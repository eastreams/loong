use super::*;
use crate::test_support::{ScopedEnv, ScopedLoongHome, unique_temp_dir};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

struct ToolTestRuntimeConfig {
    config: runtime_config::ToolRuntimeConfig,
    _runtime_home: ScopedLoongHome,
}

impl Deref for ToolTestRuntimeConfig {
    type Target = runtime_config::ToolRuntimeConfig;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl DerefMut for ToolTestRuntimeConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

impl ToolTestRuntimeConfig {
    fn into_inner(self) -> runtime_config::ToolRuntimeConfig {
        self.config
    }
}

fn test_tool_runtime_config(root: impl AsRef<Path>) -> ToolTestRuntimeConfig {
    let runtime_home = ScopedLoongHome::new("loong-tool-runtime-home");
    let config = runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::from(["echo".to_owned(), "cat".to_owned(), "ls".to_owned()]),
        file_root: Some(root.as_ref().to_path_buf()),
        messages_enabled: true,
        external_skills: runtime_config::ExternalSkillsRuntimePolicy {
            enabled: true,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: false,
        },
        ..Default::default()
    };
    ToolTestRuntimeConfig {
        config,
        _runtime_home: runtime_home,
    }
}

fn ready_bash_exec_runtime_policy() -> runtime_config::BashExecRuntimePolicy {
    let resolved_bash = which::which("bash").unwrap_or_else(|_| PathBuf::from("/bin/bash"));
    runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(resolved_bash),
        ..runtime_config::BashExecRuntimePolicy::default()
    }
}

#[cfg(all(feature = "tool-shell", unix))]
fn configured_test_bash_runtime_with_rules(
    root: &Path,
) -> (runtime_config::BashExecRuntimePolicy, PathBuf) {
    let log_path = root.join("bash-args.log");
    let runtime_path = write_fake_bash_runtime(root, "fake-bash", &log_path);
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    let rules = bash_rules::load_rules_from_dir(&rules_dir).expect("load rules");

    (
        runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(runtime_path),
            governance: runtime_config::BashGovernanceRuntimePolicy {
                rules_dir,
                rules,
                load_error: None,
            },
            ..runtime_config::BashExecRuntimePolicy::default()
        },
        log_path,
    )
}

#[cfg(all(feature = "tool-shell", unix))]
fn write_fake_bash_runtime(root: &Path, name: &str, log_path: &Path) -> PathBuf {
    let path = root.join(name);
    let script = format!(
        "#!/bin/sh\nLOG_PATH=\"{}\"\n: > \"$LOG_PATH\"\nfor arg in \"$@\"; do\n  printf '%s\\n' \"$arg\" >> \"$LOG_PATH\"\ndone\nMODE=\"${{1:-}}\"\nCOMMAND=\"${{2:-}}\"\ncase \"$MODE\" in\n  -c|-lc)\n    exec /bin/sh -c \"$COMMAND\"\n    ;;\n  *)\n    printf 'unexpected bash args: %s' \"$*\" >&2\n    exit 97\n    ;;\nesac\n",
        log_path.display()
    );
    crate::test_support::write_executable_script_atomically(&path, &script)
        .expect("write fake bash runtime");
    path
}

fn execute_tool_core_with_test_context(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if payload_uses_reserved_internal_tool_context(&request.payload) {
        with_trusted_internal_tool_payload(|| super::execute_tool_core_with_config(request, config))
    } else {
        super::execute_tool_core_with_config(request, config)
    }
}

#[test]
fn expected_tool_request_error_classifies_validation_failures() {
    assert!(super::is_expected_tool_request_error(
        "tool_not_found: unknown tool `missing`"
    ));
    assert!(super::is_expected_tool_request_error(
        "invalid_tool_lease: malformed lease"
    ));
    assert!(super::is_expected_tool_request_error(
        "tool `tool.invoke` payload._loong is reserved for trusted internal tool context; retry without that field"
    ));
    assert!(super::is_expected_tool_request_error(
        "direct_exec_ambiguous: provide either `command` or `script`, not both"
    ));
    assert!(super::is_expected_tool_request_error(
        "tool_surface_unavailable: `browser` cannot route to `managed browser actions` in this runtime; read-only browser inspection is still available"
    ));
    assert!(super::is_expected_tool_request_error(
        "web.fetch response exceeded max_bytes limit (120000 bytes); retry with a smaller `max_bytes` or a narrower web request"
    ));
}

#[test]
fn expected_tool_request_error_leaves_runtime_failures_as_warnable() {
    assert!(!super::is_expected_tool_request_error(
        "network_error: remote tool execution failed"
    ));
}

fn unique_tool_temp_dir(prefix: &str) -> PathBuf {
    unique_temp_dir(prefix)
}

const TEST_BROWSER_COMPANION_TIMEOUT_SECONDS: u64 = 120;

fn browser_companion_runtime_config(
    root: &Path,
    command: String,
) -> runtime_config::ToolRuntimeConfig {
    let mut config = test_tool_runtime_config(root).into_inner();
    config.browser_companion.enabled = true;
    config.browser_companion.ready = true;
    config.browser_companion.command = Some(command);
    config.browser_companion.timeout_seconds = TEST_BROWSER_COMPANION_TIMEOUT_SECONDS;
    config
}

mod bash_exec_tests;

#[cfg(unix)]
fn write_browser_companion_script(
    root: &Path,
    name: &str,
    stdout_body: &str,
    log_path: &Path,
) -> PathBuf {
    let path = root.join(name);
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '1.2.3\\n'\n  exit 0\nfi\nBODY=''\nIFS= read -r BODY || true\nprintf '%s' \"$BODY\" > \"{}\"\nprintf '%s' '{}'\n",
        log_path.display(),
        stdout_body.replace('\'', "'\"'\"'")
    );
    crate::test_support::write_executable_script_atomically(&path, &script)
        .expect("write browser companion script");
    path
}

#[cfg(windows)]
fn write_browser_companion_script(
    root: &Path,
    name: &str,
    stdout_body: &str,
    log_path: &Path,
) -> PathBuf {
    use std::fs;

    let path = root.join(format!("{name}.cmd"));
    let script = format!(
        "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo 1.2.3\r\n  exit /b 0\r\n)\r\nsetlocal enableextensions\r\nset /p BODY=\r\n> \"{}\" <nul set /p =%BODY%\r\necho {}\r\n",
        log_path.display(),
        stdout_body
    );
    fs::write(&path, script).expect("write browser companion script");
    path
}

#[cfg(unix)]
fn write_browser_companion_sleep_script(root: &Path, name: &str, sleep_seconds: u64) -> PathBuf {
    let path = root.join(name);
    let script = format!(
        "#!/bin/sh\nsleep {sleep_seconds}\nprintf '%s' '{{\"ok\":true,\"result\":{{\"delayed\":true}}}}'\n"
    );
    crate::test_support::write_executable_script_atomically(&path, &script)
        .expect("write browser companion sleep script");
    path
}

#[cfg(windows)]
fn write_browser_companion_sleep_script(root: &Path, name: &str, _sleep_seconds: u64) -> PathBuf {
    use std::fs;

    let path = root.join(format!("{name}.cmd"));
    let script = "@echo off\r\nping -n 3 127.0.0.1 > nul\r\necho {\"ok\":true,\"result\":{\"delayed\":true}}\r\n";
    fs::write(&path, script).expect("write browser companion sleep script");
    path
}

#[test]
fn capability_snapshot_is_deterministic() {
    let snapshot = capability_snapshot();
    assert!(snapshot.starts_with("[tool_discovery_runtime]"));
    assert!(snapshot.contains("Available tools:"));
    assert!(snapshot.contains("- read:"));
    assert!(snapshot.contains("- write:"));
    assert!(snapshot.contains("- exec:"));
    assert!(snapshot.contains("Available tools:"));
    assert!(snapshot.contains(
        "- tool.search: Discover hidden specialized tools relevant to the current task."
    ));
    assert!(snapshot.contains("- tool.invoke: Invoke a discovered hidden specialized tool using a valid lease from tool_search."));
    assert!(snapshot.contains("Hidden specialized tool tags currently discoverable:"));
    assert!(snapshot.contains("Additional specialized tools available through tool.search:"));
    assert!(snapshot.contains("Guidelines:"));
    assert!(snapshot.contains("Keep tool.search queries short and capability-focused."));
    assert!(snapshot.contains("Use read for repo inspection before shelling out."));
    assert!(snapshot.contains(
            "Use `offset` and `limit` to page through large files instead of reading everything at once."
        ));
    assert!(snapshot.contains("Use exec for normal command-line work."));
    assert!(snapshot.contains(
            "Use agent only for Loong's own approvals, sessions, delegation, provider routing, or config work."
        ));
    assert!(!snapshot.contains("shell.exec"));
    assert!(!snapshot.contains("file.read"));

    let snapshot2 = capability_snapshot();
    assert!(snapshot2.starts_with("[tool_discovery_runtime]"));
    assert!(snapshot2.contains("- tool.search:"));
}

#[test]
fn capability_snapshot_stays_compact_when_external_skills_are_installed() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-capability-snapshot-skills");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(
        &root,
        "skills/demo-skill/SKILL.md",
        "# Demo Skill\n\nUse this skill for explicit verification.\n",
    );

    let config = test_tool_runtime_config(&root);
    execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.install".to_owned(),
            payload: json!({
                "path": "skills/demo-skill"
            }),
        },
        &config,
    )
    .expect("install should succeed");

    let snapshot = capability_snapshot_with_config(&config);
    assert!(snapshot.starts_with("[tool_discovery_runtime]"));
    assert!(snapshot.contains("[available_external_skills]"));
    assert!(snapshot.contains("demo-skill"));
    assert!(snapshot.contains("external_skills.invoke"));

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(
    feature = "tool-file",
    feature = "tool-shell",
    feature = "memory-sqlite"
))]
#[test]
fn capability_snapshot_only_lists_visible_direct_and_gateway_tools() {
    let snapshot = capability_snapshot();
    assert!(snapshot.contains("Available tools:"));
    assert!(snapshot.contains("- read:"));
    assert!(snapshot.contains("- write:"));
    assert!(snapshot.contains("- exec:"));
    assert!(snapshot.contains("Available tools:"));
    assert!(snapshot.contains(
        "- tool.search: Discover hidden specialized tools relevant to the current task."
    ));
    assert!(snapshot.contains("- tool.invoke: Invoke a discovered hidden specialized tool using a valid lease from tool_search."));
    assert!(snapshot.contains("Hidden specialized tool tags currently discoverable:"));
    assert!(snapshot.contains("Additional specialized tools available through tool.search:"));
    assert!(snapshot.contains("Guidelines:"));
    assert!(snapshot.contains("Keep tool.search queries short and capability-focused."));
    assert!(snapshot.contains("Use write for new files or whole-file rewrites."));
    assert!(!snapshot.contains("claw.migrate"));
    assert!(!snapshot.contains("external_skills.fetch"));
    assert!(!snapshot.contains("file.read"));
    assert!(!snapshot.contains("shell.exec"));

    let lines: Vec<&str> = snapshot.lines().skip(1).collect();
    assert!(lines.len() >= 8);
    assert!(lines.iter().any(|line| line.starts_with("- read:")));
    assert!(lines.iter().any(|line| line.starts_with("- tool.search:")));
}

#[cfg(all(
    feature = "tool-file",
    feature = "tool-shell",
    feature = "memory-sqlite",
    feature = "tool-websearch"
))]
#[test]
fn tool_registry_returns_runtime_discoverable_tools_for_default_config() {
    let config = runtime_config::ToolRuntimeConfig::default();
    let entries = tool_registry_with_config(Some(&config));
    let names = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "approval_request_resolve",
        "approval_request_status",
        "approval_requests_list",
        "config.import",
        "delegate",
        "delegate_async",
        "external_skills.policy",
        "provider.switch",
        "session_archive",
        "session_cancel",
        "session_continue",
        "session_events",
        "session_recover",
        "session_search",
        "session_status",
        "session_tool_policy_clear",
        "session_tool_policy_set",
        "session_tool_policy_status",
        "session_wait",
        "sessions_history",
        "sessions_list",
    ]);
    assert_eq!(names, expected);
}

#[cfg(all(
    feature = "tool-file",
    feature = "tool-shell",
    feature = "memory-sqlite",
    not(feature = "tool-websearch")
))]
#[test]
fn tool_registry_returns_runtime_discoverable_tools_for_default_config_no_websearch() {
    let config = runtime_config::ToolRuntimeConfig::default();
    let entries = tool_registry_with_config(Some(&config));
    let names = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "approval_request_resolve",
        "approval_request_status",
        "approval_requests_list",
        "config.import",
        "delegate",
        "delegate_async",
        "external_skills.policy",
        "provider.switch",
        "session_archive",
        "session_cancel",
        "session_continue",
        "session_events",
        "session_recover",
        "session_search",
        "session_status",
        "session_tool_policy_clear",
        "session_tool_policy_set",
        "session_tool_policy_status",
        "session_wait",
        "sessions_history",
        "sessions_list",
    ]);

    assert_eq!(names, expected);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn tool_registry_re_exposes_session_mutation_tools_when_runtime_policy_allows_them() {
    let config = runtime_config::ToolRuntimeConfig {
        sessions_enabled: true,
        sessions_allow_mutation: true,
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let entries = tool_registry_with_config(Some(&config));
    let names = entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();

    assert!(names.contains(&"session_archive".to_owned()));
    assert!(names.contains(&"session_cancel".to_owned()));
    assert!(names.contains(&"session_continue".to_owned()));
    assert!(names.contains(&"session_recover".to_owned()));
    assert!(names.contains(&"session_tool_policy_set".to_owned()));
    assert!(names.contains(&"session_tool_policy_clear".to_owned()));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn capability_snapshot_for_view_keeps_hidden_tools_out_of_direct_surface_copy() {
    let view = ToolView::from_tool_names(["config.import", "shell.exec"]);
    let snapshot = capability_snapshot_for_view(&view);

    assert!(snapshot.contains("Available tools:"));
    assert!(snapshot.contains(
        "- tool.search: Discover hidden specialized tools relevant to the current task."
    ));
    assert!(snapshot.contains("- tool.invoke: Invoke a discovered hidden specialized tool using a valid lease from tool_search."));
    assert!(!snapshot.contains("- config.import:"));
    assert!(!snapshot.contains("- shell.exec:"));
    assert!(snapshot.contains("- exec:"));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn try_provider_tool_definitions_for_view_exposes_gateway_when_no_direct_tool_is_visible() {
    let view = ToolView::from_tool_names(["config.import"]);
    let defs = try_provider_tool_definitions_for_view(&view)
        .expect("restricted runtime view should still expose the discovery gateway");
    let names: Vec<&str> = defs
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .collect();

    assert_eq!(names, vec!["tool_invoke", "tool_search"]);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn runtime_tool_view_hides_session_mutation_tools_when_explicitly_disabled() {
    let mut config = crate::config::ToolConfig::default();
    config.sessions.allow_mutation = false;
    let view = runtime_tool_view_for_config(&config);

    for tool_name in [
        "approval_request_resolve",
        "approval_request_status",
        "approval_requests_list",
        "delegate",
        "delegate_async",
        "session_events",
        "session_tool_policy_status",
        "session_search",
        "session_status",
        "session_wait",
        "sessions_history",
        "sessions_list",
        "browser.click",
        "browser.extract",
        "browser.open",
    ] {
        assert!(
            view.contains(tool_name),
            "expected runtime view to include `{tool_name}`"
        );
    }

    for tool_name in [
        "session_archive",
        "session_cancel",
        "session_continue",
        "session_recover",
    ] {
        assert!(
            !view.contains(tool_name),
            "expected runtime view to hide `{tool_name}` by default"
        );
    }

    let tool_name = "sessions_send";
    assert!(
        !view.contains(tool_name),
        "expected runtime view to keep `{tool_name}` hidden"
    );
    assert!(view.contains("web.fetch"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn runtime_tool_view_re_exposes_session_mutation_tools_when_enabled() {
    let mut config = crate::config::ToolConfig::default();
    config.sessions.enabled = true;
    config.sessions.allow_mutation = true;

    let view = runtime_tool_view_for_config(&config);

    assert!(view.contains("session_archive"));
    assert!(view.contains("session_cancel"));
    assert!(view.contains("session_continue"));
    assert!(view.contains("session_recover"));
    assert!(view.contains("session_tool_policy_set"));
    assert!(view.contains("session_tool_policy_clear"));
}

#[test]
fn runtime_tool_view_hides_web_fetch_when_disabled() {
    let mut config = crate::config::ToolConfig::default();
    config.web.enabled = false;

    let root_view = runtime_tool_view_for_config(&config);
    assert!(!root_view.contains("web.fetch"));
}

#[test]
fn runtime_tool_view_hides_web_search_when_disabled() {
    let mut config = crate::config::ToolConfig::default();
    config.web_search.enabled = false;

    let root_view = runtime_tool_view_for_config(&config);
    assert!(!root_view.contains("web.search"));
}

#[test]
fn runtime_tool_view_hides_browser_when_disabled() {
    let mut config = crate::config::ToolConfig::default();
    config.browser.enabled = false;

    let root_view = runtime_tool_view_for_config(&config);
    assert!(!root_view.contains("browser.open"));
    assert!(!root_view.contains("browser.extract"));
    assert!(!root_view.contains("browser.click"));
}

#[test]
fn runtime_tool_view_respects_explicit_external_skills_toggle() {
    let config = crate::config::ToolConfig::default();

    let disabled_view = runtime_tool_view_for_config(&config);
    assert!(!disabled_view.contains("external_skills.fetch"));
    assert!(!disabled_view.contains("external_skills.invoke"));
    assert!(!disabled_view.contains("external_skills.list"));

    let enabled_view = runtime_tool_view_for_config_with_external_skills(&config, true);
    assert!(enabled_view.contains("external_skills.fetch"));
    assert!(enabled_view.contains("external_skills.invoke"));
    assert!(enabled_view.contains("external_skills.list"));
}

#[test]
fn runtime_tool_view_with_runtime_config_uses_runtime_external_skills_policy() {
    let runtime_config = runtime_config::ToolRuntimeConfig {
        external_skills: runtime_config::ExternalSkillsRuntimePolicy {
            enabled: true,
            ..runtime_config::ExternalSkillsRuntimePolicy::default()
        },
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let view = runtime_tool_view_with_runtime_config(&ToolConfig::default(), &runtime_config);

    assert!(view.contains("external_skills.fetch"));
    assert!(view.contains("external_skills.invoke"));
    assert!(view.contains("external_skills.list"));
}

#[test]
fn capability_snapshot_with_config_uses_runtime_enabled_tool_view() {
    let config = runtime_config::ToolRuntimeConfig {
        sessions_enabled: false,
        sessions_allow_mutation: false,
        messages_enabled: false,
        delegate_enabled: false,
        browser: runtime_config::BrowserRuntimePolicy {
            enabled: false,
            max_sessions: 8,
            max_links: 40,
            max_text_chars: 6000,
        },
        web_fetch: runtime_config::WebFetchRuntimePolicy {
            enabled: false,
            ..runtime_config::WebFetchRuntimePolicy::default()
        },
        external_skills: runtime_config::ExternalSkillsRuntimePolicy {
            enabled: false,
            ..runtime_config::ExternalSkillsRuntimePolicy::default()
        },
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let snapshot = capability_snapshot_with_config(&config);
    assert!(!snapshot.contains("- browser.open:"));
    assert!(!snapshot.contains("- web.fetch:"));
    assert!(!snapshot.contains("- delegate:"));
    assert!(!snapshot.contains("- external_skills.fetch:"));
}

#[test]
fn runtime_tool_view_exposes_delegate_tools_with_depth_budget_only() {
    let config = crate::config::ToolConfig::default();

    let root_view = runtime_tool_view_for_config(&config);
    assert!(root_view.contains("delegate"));
    assert!(root_view.contains("delegate_async"));

    let child_view = delegate_child_tool_view_for_config(&config);
    assert!(!child_view.contains("delegate"));
    assert!(!child_view.contains("delegate_async"));

    let depth_budgeted_child = delegate_child_tool_view_for_config_with_delegate(&config, true);
    assert!(depth_budgeted_child.contains("delegate"));
    assert!(depth_budgeted_child.contains("delegate_async"));
}

#[test]
fn runtime_tool_view_exposes_sessions_send_only_when_messages_enabled() {
    let default_root_view = runtime_tool_view_for_config(&crate::config::ToolConfig::default());
    assert!(!default_root_view.contains("sessions_send"));

    let mut config = crate::config::ToolConfig::default();
    config.messages.enabled = true;

    let root_view = runtime_tool_view_for_config(&config);
    assert!(root_view.contains("sessions_send"));

    let child_view = delegate_child_tool_view_for_config(&config);
    assert!(!child_view.contains("sessions_send"));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn delegate_child_tool_view_hides_shell_by_default() {
    let view = delegate_child_tool_view_for_config(&crate::config::ToolConfig::default());

    assert!(view.contains("file.read"));
    assert!(view.contains("file.write"));
    assert!(!view.contains("shell.exec"));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn delegate_child_tool_view_can_allow_shell_when_enabled() {
    let mut config = crate::config::ToolConfig::default();
    config.delegate.allow_shell_in_child = true;

    let view = delegate_child_tool_view_for_config(&config);

    assert!(view.contains("file.read"));
    assert!(view.contains("file.write"));
    assert!(view.contains("shell.exec"));
}

#[cfg(all(
    feature = "tool-file",
    feature = "tool-shell",
    feature = "memory-sqlite"
))]
#[test]
fn provider_tool_definitions_are_stable_and_cover_direct_surface() {
    let config = runtime_config::ToolRuntimeConfig::default();
    let defs = provider_tool_definitions_with_config(Some(&config));
    let expected_names = vec![
        "browser",
        "exec",
        "read",
        "tool_invoke",
        "tool_search",
        "web",
        "write",
    ];
    assert_eq!(defs.len(), expected_names.len());

    let names: Vec<&str> = defs
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, expected_names);

    for item in &defs {
        assert_eq!(item["type"], "function");
        assert_eq!(item["function"]["parameters"]["type"], "object");

        let parameters = item["function"]["parameters"]
            .as_object()
            .expect("provider parameters should be an object");
        for combinator_key in ["allOf", "anyOf", "oneOf"] {
            assert!(
                !parameters.contains_key(combinator_key),
                "provider parameters should not expose `{combinator_key}`: {item:?}"
            );
        }
    }

    let tool_search = defs
        .iter()
        .find(|item| {
            item.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("tool_search")
        })
        .expect("tool_search definition should exist");
    let tool_search_properties = tool_search["function"]["parameters"]["properties"]
        .as_object()
        .expect("tool_search properties should be an object");
    let tool_search_required = tool_search["function"]["parameters"]["required"]
        .as_array()
        .expect("required should be an array");

    assert!(tool_search_properties.contains_key("query"));
    assert!(tool_search_properties.contains_key("exact_tool_id"));
    assert!(!tool_search_required.contains(&Value::String("query".to_owned())));
    assert!(
        tool_search["function"]["parameters"].get("anyOf").is_none(),
        "anyOf removed for OpenAI-compatible provider compatibility"
    );

    let web = defs
        .iter()
        .find(|item| {
            item.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("web")
        })
        .expect("web definition should exist");
    let web_properties = web["function"]["parameters"]["properties"]
        .as_object()
        .expect("web properties should be an object");
    assert!(web_properties.contains_key("url"));
    assert!(web_properties.contains_key("query"));
    assert!(web_properties.contains_key("method"));
    let web_provider_description = web_properties["provider"]["description"]
        .as_str()
        .expect("web provider description should be text");
    assert!(web_provider_description.contains("`web { query }` / `web.search`"));
    assert!(web_provider_description.contains("plain URL fetch/request mode"));

    let browser = defs
        .iter()
        .find(|item| {
            item.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("browser")
        })
        .expect("browser definition should exist");
    let browser_properties = browser["function"]["parameters"]["properties"]
        .as_object()
        .expect("browser properties should be an object");
    assert!(browser_properties.contains_key("url"));
    assert!(browser_properties.contains_key("session_id"));
    assert!(browser_properties.contains_key("text"));
    assert!(browser_properties.contains_key("condition"));
    assert!(browser_properties.contains_key("timeout_ms"));
    assert!(
        !browser_properties.contains_key("action"),
        "browser should stay payload-shape-driven instead of teaching sub-action names"
    );
}

#[test]
fn provider_exposed_tool_gate_covers_direct_and_gateway_tools() {
    assert!(is_provider_exposed_tool_name("read"));
    assert!(is_provider_exposed_tool_name("write"));
    assert!(is_provider_exposed_tool_name("exec"));
    assert!(is_provider_exposed_tool_name("tool.search"));
    assert!(is_provider_exposed_tool_name("tool.invoke"));
    assert!(!is_provider_exposed_tool_name("file.read"));
    assert!(!is_provider_exposed_tool_name("shell.exec"));
}

#[cfg(feature = "tool-http")]
#[test]
fn provider_tool_definitions_include_http_request_when_enabled() {
    let catalog = tool_catalog();
    let http_request_descriptor = catalog
        .descriptor(HTTP_REQUEST_TOOL_NAME)
        .expect("http.request should be in the catalog");
    let definition = http_request_descriptor.provider_definition();
    let properties = definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("http.request properties");
    assert!(properties.contains_key("url"));
    assert!(properties.contains_key("method"));
    assert!(properties.contains_key("headers"));
    assert!(properties.contains_key("content_type"));
    assert!(properties.contains_key("max_bytes"));
}

#[test]
fn provider_tool_definitions_include_web_fetch_when_enabled() {
    let catalog = tool_catalog();
    let web_fetch_descriptor = catalog
        .descriptor("web.fetch")
        .expect("web.fetch should be in the catalog");
    let def = web_fetch_descriptor.provider_definition();
    let properties = def["function"]["parameters"]["properties"]
        .as_object()
        .expect("web_fetch properties");
    assert!(properties.contains_key("url"));
    assert!(properties.contains_key("mode"));
    assert!(properties.contains_key("max_bytes"));
    assert_eq!(
        properties["max_bytes"]["maximum"],
        json!(5 * 1024 * 1024),
        "web.fetch schema should advertise the compile-time hard cap instead of the default runtime limit"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_catalog_exposes_timeout_ms() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("shell.exec")
        .expect("shell.exec should be in the catalog");
    let definition = descriptor.provider_definition();
    let properties = definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("shell.exec parameters");

    assert!(
        properties.contains_key("timeout_ms"),
        "shell.exec schema should expose timeout_ms parameter"
    );

    let entry = catalog::find_tool_catalog_entry("shell.exec")
        .expect("shell.exec should be in catalog entries");
    assert!(
        entry
            .argument_hint
            .split(',')
            .any(|part| part == "timeout_ms?:integer"),
        "shell.exec argument hint should expose timeout_ms"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn file_write_catalog_exposes_overwrite_flag() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("file.write")
        .expect("file.write should be in the catalog");
    let definition = descriptor.provider_definition();
    let properties = definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("file.write parameters");
    let required_fields = definition["function"]["parameters"]["required"].as_array();

    assert!(
        properties.contains_key("overwrite"),
        "file.write schema should expose overwrite parameter"
    );
    assert!(
        required_fields
            .is_none_or(|fields| !fields.contains(&Value::String("overwrite".to_owned()))),
        "file.write schema should keep overwrite optional"
    );

    let entry = catalog::find_tool_catalog_entry("file.write")
        .expect("file.write should be in catalog entries");
    assert!(
        entry
            .argument_hint
            .split(',')
            .any(|part| part == "overwrite?:boolean"),
        "file.write argument hint should expose overwrite"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn file_edit_catalog_exposes_exact_edit_blocks() {
    let catalog = tool_catalog();
    let direct_descriptor = catalog
        .descriptor("write")
        .expect("write should be in the catalog");
    let direct_definition = direct_descriptor.provider_definition();
    let direct_properties = direct_definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("write parameters");
    let direct_any_of = direct_definition["function"]["parameters"]["anyOf"]
        .as_array()
        .expect("write anyOf");

    assert!(
        direct_properties.contains_key("edits"),
        "write schema should expose exact edit blocks"
    );
    assert!(
        direct_descriptor.argument_hint().contains("edits?:array"),
        "write argument hint should expose edits"
    );
    assert!(
        direct_descriptor
            .parameter_types()
            .contains(&("edits", "array")),
        "write parameter types should expose edits"
    );
    assert!(
        direct_any_of
            .iter()
            .any(|branch| branch["required"] == json!(["edits"])),
        "write anyOf should include edits mode"
    );

    let file_edit_descriptor = catalog
        .descriptor("file.edit")
        .expect("file.edit should be in the catalog");
    let file_edit_definition = file_edit_descriptor.provider_definition();
    let file_edit_properties = file_edit_definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("file.edit parameters");
    let file_edit_any_of = file_edit_definition["function"]["parameters"]["anyOf"]
        .as_array()
        .expect("file.edit anyOf");

    assert!(
        file_edit_properties.contains_key("edits"),
        "file.edit schema should expose exact edit blocks"
    );
    assert!(
        file_edit_descriptor
            .argument_hint()
            .contains("edits?:array"),
        "file.edit argument hint should expose edits"
    );
    assert!(
        file_edit_descriptor
            .parameter_types()
            .contains(&("edits", "array")),
        "file.edit parameter types should expose edits"
    );
    assert_eq!(file_edit_descriptor.required_fields(), vec!["path"]);
    assert!(
        file_edit_any_of
            .iter()
            .any(|branch| branch["required"] == json!(["edits"])),
        "file.edit anyOf should include edits mode"
    );
}

#[cfg(feature = "tool-websearch")]
#[test]
fn tool_registry_hides_web_search_when_runtime_disabled() {
    let config = runtime_config::ToolRuntimeConfig {
        web_search: runtime_config::WebSearchRuntimePolicy {
            enabled: false,
            ..runtime_config::WebSearchRuntimePolicy::default()
        },
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let entries = tool_registry_with_config(Some(&config));

    assert!(
        !entries.iter().any(|entry| entry.name == "web.search"),
        "runtime-disabled web.search should not appear in tool registry"
    );
}

#[test]
fn provider_tool_definitions_include_browser_open_when_enabled() {
    let catalog = tool_catalog();
    let browser_open_descriptor = catalog
        .descriptor("browser.open")
        .expect("browser.open should be in the catalog");
    let def = browser_open_descriptor.provider_definition();
    let properties = def["function"]["parameters"]["properties"]
        .as_object()
        .expect("browser_open properties");
    assert!(properties.contains_key("url"));
    assert!(!properties.contains_key("session_id"));
    assert!(properties.contains_key("max_bytes"));
}

#[test]
fn provider_tool_definitions_trim_web_query_mode_when_search_is_runtime_disabled() {
    let config = runtime_config::ToolRuntimeConfig {
        web_search: runtime_config::WebSearchRuntimePolicy {
            enabled: false,
            ..runtime_config::WebSearchRuntimePolicy::default()
        },
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let defs = provider_tool_definitions_with_config(Some(&config));
    let web = defs
        .iter()
        .find(|item| {
            item.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("web")
        })
        .expect("web definition should exist");
    let properties = web["function"]["parameters"]["properties"]
        .as_object()
        .expect("web properties should be an object");
    assert!(properties.contains_key("url"));
    assert!(!properties.contains_key("query"));
    assert!(!properties.contains_key("provider"));
    assert!(!properties.contains_key("max_results"));
    assert_eq!(web["function"]["parameters"]["required"], json!(["url"]));
}

#[test]
fn runtime_tool_search_entries_trim_web_search_mode_when_query_mode_is_disabled() {
    let config = runtime_config::ToolRuntimeConfig {
        web_search: runtime_config::WebSearchRuntimePolicy {
            enabled: false,
            ..runtime_config::WebSearchRuntimePolicy::default()
        },
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let entries = runtime_tool_search_entries(&config, None, false);
    let web = entries
        .iter()
        .find(|entry| entry.tool_id == "web")
        .expect("web direct search entry");
    assert!(web.summary.contains("Fetch a URL or send HTTP requests"));
    assert!(!web.argument_hint.contains("query"));
    assert!(web.search_hint.contains("query search mode is unavailable"));
    assert!(
        web.usage_guidance
            .as_deref()
            .is_some_and(|guidance| guidance.contains("Query search mode is unavailable"))
    );
}

#[test]
fn canonical_tool_name_maps_known_aliases() {
    assert_eq!(canonical_tool_name("tool_search"), "tool.search");
    assert_eq!(canonical_tool_name("tool_invoke"), "tool.invoke");
    assert_eq!(canonical_tool_name("claw.migrate"), "config.import");
    assert_eq!(canonical_tool_name("claw_migrate"), "config.import");
    assert_eq!(canonical_tool_name("config_import"), "config.import");
    assert_eq!(
        canonical_tool_name("external_skills_policy"),
        "external_skills.policy"
    );
    assert_eq!(
        canonical_tool_name("external_skills_fetch"),
        "external_skills.fetch"
    );
    assert_eq!(canonical_tool_name("file_read"), "file.read");
    assert_eq!(canonical_tool_name("file_write"), "file.write");
    assert_eq!(canonical_tool_name("provider_switch"), "provider.switch");
    assert_eq!(canonical_tool_name("browser_open"), "browser.open");
    assert_eq!(canonical_tool_name("browser_extract"), "browser.extract");
    assert_eq!(canonical_tool_name("browser_click"), "browser.click");
    assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
    assert_eq!(canonical_tool_name("shell"), "shell.exec");
    assert_eq!(canonical_tool_name("web_fetch"), "web.fetch");
    assert_eq!(canonical_tool_name("feishu_whoami"), "feishu.whoami");
    assert_eq!(
        canonical_tool_name("feishu_doc_create"),
        "feishu.doc.create"
    );
    assert_eq!(
        canonical_tool_name("feishu_doc_append"),
        "feishu.doc.append"
    );
    assert_eq!(canonical_tool_name("feishu_doc_read"), "feishu.doc.read");
    assert_eq!(
        canonical_tool_name("feishu_messages_history"),
        "feishu.messages.history"
    );
    assert_eq!(
        canonical_tool_name("feishu_messages_get"),
        "feishu.messages.get"
    );
    assert_eq!(
        canonical_tool_name("feishu_messages_resource_get"),
        "feishu.messages.resource.get"
    );
    assert_eq!(
        canonical_tool_name("feishu_messages_search"),
        "feishu.messages.search"
    );
    assert_eq!(
        canonical_tool_name("feishu_messages_send"),
        "feishu.messages.send"
    );
    assert_eq!(
        canonical_tool_name("feishu_messages_reply"),
        "feishu.messages.reply"
    );
    assert_eq!(
        canonical_tool_name("feishu_calendar_list"),
        "feishu.calendar.list"
    );
    assert_eq!(
        canonical_tool_name("feishu_calendar_freebusy"),
        "feishu.calendar.freebusy"
    );
    assert_eq!(canonical_tool_name("file.read"), "file.read");
}

#[cfg(feature = "tool-file")]
#[test]
fn runtime_tool_view_hides_memory_tools_when_memory_corpus_is_empty() {
    let root = unique_tool_temp_dir("loongclaw-memory-tool-view-empty");

    std::fs::create_dir_all(&root).expect("create root dir");

    let config = test_tool_runtime_config(root);
    let tool_view = runtime_tool_view_for_runtime_config(&config);

    assert!(!tool_view.contains("memory_search"));
    assert!(!tool_view.contains("memory_get"));
}

#[cfg(feature = "tool-file")]
#[test]
fn runtime_tool_view_includes_memory_tools_when_memory_corpus_exists() {
    let root = unique_tool_temp_dir("loongclaw-memory-tool-view-visible");
    let memory_path = root.join("MEMORY.md");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(
        &memory_path,
        "# Durable Notes\nDeploy freeze window is Friday.\n",
    )
    .expect("write root memory");

    let config = test_tool_runtime_config(root);
    let tool_view = runtime_tool_view_for_runtime_config(&config);

    assert!(tool_view.contains("memory_search"));
    assert!(tool_view.contains("memory_get"));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_returns_direct_results_for_common_file_queries() {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-tool-search-{nanos}"));
    fs::create_dir_all(&root).expect("create fixture root");
    fs::write(root.join("README.md"), "hello tool search").expect("write fixture");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "read repo file",
                "limit": 3
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    assert_eq!(outcome.status, "ok");
    let results = outcome.payload["results"].as_array().expect("results");
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .all(|entry| entry["tool_id"] != "tool.search")
    );
    assert!(results.iter().any(|entry| entry["tool_id"] == "read"));
    assert!(results.iter().all(|entry| {
        if entry["tool_id"] != "read" {
            return true;
        }
        entry.get("lease").is_none()
    }));

    fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_surfaces_memory_tools_when_memory_corpus_is_available() {
    let root = unique_tool_temp_dir("loongclaw-memory-tool-search");
    let memory_dir = root.join("memory");

    std::fs::create_dir_all(&memory_dir).expect("create memory dir");
    std::fs::write(root.join("MEMORY.md"), "Deploy freeze window: Friday.\n")
        .expect("write root memory");
    std::fs::write(
        memory_dir.join("2026-03-23.md"),
        "Migration starts tomorrow.\n",
    )
    .expect("write daily log");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "search memory recall durable notes",
                "limit": 6
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");

    assert!(results.iter().any(|entry| entry["tool_id"] == "memory"));
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_hides_memory_tools_when_memory_corpus_is_empty() {
    let root = unique_tool_temp_dir("loongclaw-memory-tool-search-empty");

    std::fs::create_dir_all(&root).expect("create root dir");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "search memory recall durable notes",
                "limit": 6
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");

    assert!(
        results
            .iter()
            .all(|entry| entry["tool_id"] != "memory_search")
    );
    assert!(results.iter().all(|entry| entry["tool_id"] != "memory_get"));
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_search_tool_returns_structured_hits_from_workspace_memory_files() {
    let root = unique_tool_temp_dir("loongclaw-memory-search");
    let memory_dir = root.join("memory");

    std::fs::create_dir_all(&memory_dir).expect("create memory dir");
    std::fs::write(
        root.join("MEMORY.md"),
        "# Durable Notes\nDeploy freeze window is Friday.\n",
    )
    .expect("write root memory");
    std::fs::write(
        memory_dir.join("2026-03-23.md"),
        "Customer migration starts tomorrow.\n",
    )
    .expect("write daily log");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_search".to_owned(),
            payload: json!({
                "query": "deploy freeze window",
                "max_results": 4
            }),
        },
        &config,
    )
    .expect("memory search should succeed");

    assert_eq!(outcome.status, "ok");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(!results.is_empty());
    assert!(results.iter().any(|entry| entry["path"] == "MEMORY.md"));
    assert!(
        results
            .iter()
            .all(|entry| entry["start_line"].as_u64().is_some()),
        "expected structured line spans: {results:?}"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["end_line"].as_u64().is_some()),
        "expected structured line spans: {results:?}"
    );
    assert!(
        results.iter().all(|entry| entry["snippet"]
            .as_str()
            .is_some_and(|value| !value.is_empty())),
        "expected non-empty snippets: {results:?}"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["source"] == "workspace_file"),
        "expected workspace-file results only: {results:?}"
    );
    assert!(
        results.iter().all(|entry| {
            entry["provenance"]["memory_system_id"] == "builtin"
                && entry["provenance"]["source_kind"] == "workspace_document"
                && entry["provenance"]["recall_mode"] == "operator_inspection"
        }),
        "expected structured operator-inspection provenance: {results:?}"
    );
    assert!(
        results.iter().all(|entry| {
            entry["metadata"]["record_status"] == "active"
                && entry["metadata"]["body_line_offset"].as_u64().is_some()
        }),
        "expected structured workspace metadata: {results:?}"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_get_tool_returns_bounded_line_window_from_memory_file() {
    let root = unique_tool_temp_dir("loongclaw-memory-get");
    let memory_path = root.join("MEMORY.md");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(
        &memory_path,
        "line one\nline two\nline three\nline four\nline five\n",
    )
    .expect("write root memory");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "MEMORY.md",
                "from": 2,
                "lines": 2
            }),
        },
        &config,
    )
    .expect("memory get should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["path"], "MEMORY.md");
    assert_eq!(outcome.payload["start_line"], 2);
    assert_eq!(outcome.payload["end_line"], 3);
    assert_eq!(outcome.payload["text"], "line two\nline three");
    assert_eq!(outcome.payload["provenance"]["memory_system_id"], "builtin");
    assert_eq!(
        outcome.payload["provenance"]["source_kind"],
        "workspace_document"
    );
    assert_eq!(outcome.payload["provenance"]["scope"], "workspace");
    assert_eq!(
        outcome.payload["provenance"]["recall_mode"],
        "operator_inspection"
    );
    assert_eq!(outcome.payload["metadata"]["record_status"], "active");
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_get_tool_uses_selected_memory_system_id_in_provenance() {
    let root = unique_tool_temp_dir("loongclaw-memory-get-selected-system");
    let memory_path = root.join("MEMORY.md");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(&memory_path, "line one\nline two\n").expect("write root memory");

    let mut config = test_tool_runtime_config(root);
    config.selected_memory_system_id = "workspace_recall".to_owned();

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "MEMORY.md",
                "from": 1,
                "lines": 1
            }),
        },
        &config,
    )
    .expect("memory get should succeed");

    assert_eq!(
        outcome.payload["provenance"]["memory_system_id"],
        "workspace_recall"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_get_tool_reads_requested_window_without_loading_invalid_tail() {
    let root = unique_tool_temp_dir("loongclaw-memory-get-invalid-tail");
    let memory_path = root.join("MEMORY.md");
    let mut bytes = b"line one\nline two\n".to_vec();

    bytes.push(0xff);
    bytes.push(0xfe);

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(&memory_path, bytes).expect("write root memory");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "MEMORY.md",
                "from": 1,
                "lines": 2
            }),
        },
        &config,
    )
    .expect("memory get should ignore invalid bytes beyond requested window");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["start_line"], 1);
    assert_eq!(outcome.payload["end_line"], 2);
    assert_eq!(outcome.payload["text"], "line one\nline two");
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_search_tool_rejects_invalid_max_results_values() {
    let root = unique_tool_temp_dir("loongclaw-memory-search-invalid-max-results");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(root.join("MEMORY.md"), "deploy freeze window\n").expect("write memory");

    let config = test_tool_runtime_config(root);
    let non_numeric_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_search".to_owned(),
            payload: json!({
                "query": "deploy",
                "max_results": "3"
            }),
        },
        &config,
    )
    .expect_err("non-numeric max_results should fail");
    let out_of_range_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_search".to_owned(),
            payload: json!({
                "query": "deploy",
                "max_results": 0
            }),
        },
        &config,
    )
    .expect_err("out-of-range max_results should fail");

    assert!(non_numeric_error.contains("payload.max_results"));
    assert!(out_of_range_error.contains("payload.max_results"));
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_get_tool_rejects_invalid_window_arguments() {
    let root = unique_tool_temp_dir("loongclaw-memory-get-invalid-window");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(root.join("MEMORY.md"), "line one\nline two\n").expect("write memory");

    let config = test_tool_runtime_config(root);
    let invalid_from_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "MEMORY.md",
                "from": "2"
            }),
        },
        &config,
    )
    .expect_err("non-numeric from should fail");
    let invalid_lines_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "MEMORY.md",
                "lines": 0
            }),
        },
        &config,
    )
    .expect_err("out-of-range lines should fail");

    assert!(invalid_from_error.contains("payload.from"));
    assert!(invalid_lines_error.contains("payload.lines"));
}

#[cfg(feature = "tool-file")]
#[test]
fn memory_get_tool_hides_non_corpus_file_existence() {
    let root = unique_tool_temp_dir("loongclaw-memory-get-corpus-boundary");

    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::write(root.join("MEMORY.md"), "line one\nline two\n").expect("write memory");
    std::fs::write(root.join("README.md"), "not in corpus\n").expect("write readme");

    let config = test_tool_runtime_config(root);
    let existing_non_corpus_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "README.md"
            }),
        },
        &config,
    )
    .expect_err("existing non-corpus file should fail");
    let missing_non_corpus_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "memory_get".to_owned(),
            payload: json!({
                "path": "missing.md"
            }),
        },
        &config,
    )
    .expect_err("missing non-corpus file should fail");

    assert!(existing_non_corpus_error.contains("not part of the workspace durable memory corpus"));
    assert!(missing_non_corpus_error.contains("not part of the workspace durable memory corpus"));
    assert!(!existing_non_corpus_error.contains("not an existing file"));
    assert!(!missing_non_corpus_error.contains("not an existing file"));
}

mod search_and_shell;

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_result_includes_search_hint_and_schema_preview() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-card-metadata");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "run shell command",
                "limit": 3
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    let shell_entry = results
        .iter()
        .find(|entry| entry["tool_id"] == "exec")
        .expect("direct exec should be discoverable");

    assert!(shell_entry["search_hint"].as_str().is_some());
    assert!(shell_entry["schema_preview"].is_object());

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_accepts_keywords_array_payloads() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-keywords-array");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "keywords": ["run", "shell", "command"],
                "limit": 3
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");

    assert!(!results.is_empty());
    assert_eq!(outcome.payload["query"], json!("run shell command"));
    assert_eq!(results[0]["tool_id"], "exec");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-file", feature = "tool-webfetch"))]
#[test]
fn tool_search_uses_schema_derived_terms_for_web_fetch_modes() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-schema-derived");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "raw_text",
                "limit": 6
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().any(|entry| entry["tool_id"] == "web"),
        "schema-derived enum terms should make direct web discoverable: {results:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-file", feature = "tool-websearch"))]
#[test]
fn tool_search_matches_prompt_style_queries_across_tool_surfaces() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-surface-prompts");
    let memory_dir = root.join("memory");

    std::fs::create_dir_all(&memory_dir).expect("create memory dir");
    std::fs::write(root.join("MEMORY.md"), "deploy freeze window\n").expect("write root memory");

    let config = test_tool_runtime_config(root.clone());
    let cases = vec![
        ("edit file", "write"),
        ("read repo file", "read"),
        ("search memory notes", "memory"),
        ("search the web", "web"),
        ("install skill", "skills"),
        ("switch provider", "agent"),
    ];

    for (query, expected_tool) in cases {
        let payload = json!({
            "query": query,
            "limit": 8
        });
        let request = ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload,
        };
        let outcome =
            execute_tool_core_with_config(request, &config).expect("tool search should succeed");
        let results = outcome.payload["results"].as_array().expect("results");
        let expected_entry = results
            .iter()
            .find(|entry| entry["tool_id"] == expected_tool);

        assert!(
            expected_entry.is_some(),
            "expected `{expected_tool}` for prompt-style query `{query}`, got {results:?}"
        );

        let expected_entry = expected_entry.expect("expected tool entry");
        let why = expected_entry["why"].as_array().expect("why");
        let used_coarse_fallback = why
            .iter()
            .any(|reason| reason.as_str() == Some("coarse_fallback"));

        assert!(
            !used_coarse_fallback,
            "expected semantic match for `{query}`, got coarse fallback: {expected_entry:?}"
        );
    }

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_uses_coarse_listing_fallback_when_query_is_missing() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-missing-query");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "limit": 4
            }),
        },
        &config,
    )
    .expect("tool search should succeed without a query");

    let diagnostics = &outcome.payload["diagnostics"];
    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        !results.is_empty(),
        "missing-query fallback should still list runtime-visible tools"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["why"].as_array().is_some_and(|why| why
                .iter()
                .any(|reason| reason.as_str() == Some("coarse_fallback")))),
        "missing-query fallback should explain its coarse listing mode: {results:?}"
    );
    assert_eq!(diagnostics["reason"], "coarse_fallback");
    assert_eq!(diagnostics["query"], "");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn direct_write_routes_exact_edit_blocks_to_file_edit() {
    let root = unique_tool_temp_dir("loongclaw-direct-write-edit-blocks");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let target = root.join("notes.txt");
    std::fs::write(&target, "alpha\nbeta\ngamma\n").expect("seed target file");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "write".to_owned(),
            payload: json!({
                "path": "notes.txt",
                "edits": [
                    {"old_text": "alpha", "new_text": "ALPHA"},
                    {"old_text": "gamma", "new_text": "GAMMA"}
                ]
            }),
        },
        &config,
    )
    .expect("direct write edit mode should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["edit_blocks_applied"], 2);
    let updated = std::fs::read_to_string(&target).expect("read updated target");
    assert_eq!(updated, "ALPHA\nbeta\nGAMMA\n");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_prefers_direct_write_for_write_queries() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-write-query");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "write content into a file",
                "limit": 3
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    let first_tool_id = results
        .first()
        .and_then(|entry| entry.get("tool_id"))
        .and_then(Value::as_str);
    assert_eq!(first_tool_id, Some("write"));

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_accepts_keywords_array_queries() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-keywords-query");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "keywords": ["write", "file"],
                "limit": 3
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let query = outcome.payload["query"].as_str();
    let results = outcome.payload["results"].as_array().expect("results");
    let first_tool_id = results
        .first()
        .and_then(|entry| entry.get("tool_id"))
        .and_then(Value::as_str);

    assert_eq!(query, Some("write file"));
    assert_eq!(first_tool_id, Some("write"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn capability_snapshot_summarizes_hidden_tags_without_tool_names() {
    let snapshot = capability_snapshot();
    let hidden_tag_line = snapshot
        .lines()
        .find(|line| line.starts_with("Hidden specialized tool tags currently discoverable:"))
        .expect("hidden tool tag line");

    assert!(
        !hidden_tag_line.contains("provider.switch"),
        "capability summary should expose tags, not tool names: {hidden_tag_line}"
    );
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn runtime_discoverable_tool_surface_summary_groups_visible_direct_and_hidden_surfaces() {
    let root = unique_tool_temp_dir("loongclaw-tool-surface-summary");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let summary = runtime_discoverable_tool_surface_summary_with_config(
        &config,
        Some(&runtime_tool_view_for_runtime_config(&config)),
    );

    assert!(summary.hidden_tool_count > 0);
    assert!(summary.visible_direct_tools.contains(&"read".to_owned()));
    assert!(summary.visible_direct_tools.contains(&"write".to_owned()));
    assert!(summary.visible_direct_tools.contains(&"exec".to_owned()));
    assert!(!summary.hidden_tags.is_empty());

    let agent_surface = summary
        .hidden_surfaces
        .iter()
        .find(|surface| surface.surface_id == "agent")
        .expect("agent surface");
    assert!(agent_surface.tool_ids.contains(&"agent".to_owned()));

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn direct_web_routes_low_level_request_fields_through_web_surface() {
    assert_eq!(
        route_direct_web_tool_name(&json!({
            "url": "https://example.com/api",
            "method": "POST"
        }))
        .expect("request-mode web routing should succeed"),
        "http.request"
    );
    assert_eq!(
        route_direct_web_tool_name(&json!({
            "query": "rust http client"
        }))
        .expect("search-mode web routing should succeed"),
        "web.search"
    );
}

#[test]
fn direct_web_runtime_routing_keeps_network_mode_when_search_mode_is_unavailable() {
    let view = ToolView::from_tool_names(["web.fetch", "http.request"]);
    let error = route_direct_web_tool_name_for_view(
        &json!({
            "query": "rust http client"
        }),
        &view,
    )
    .expect_err("query mode should be unavailable in this runtime");
    assert!(error.contains("ordinary network access still works"));

    let fallback = route_direct_web_tool_name_for_view(
        &json!({
            "url": "https://example.com/docs"
        }),
        &ToolView::from_tool_names(["http.request"]),
    )
    .expect("plain URL web mode should fall back to http.request when fetch mode is absent");
    assert_eq!(fallback, "http.request");
}

#[cfg(feature = "tool-browser")]
#[test]
fn direct_browser_routes_managed_browser_actions_through_browser_surface() {
    assert_eq!(
        route_direct_browser_tool_name(&json!({
            "action": "start",
            "url": "https://example.com"
        }))
        .expect("managed browser start routing should succeed"),
        "browser.companion.session.start"
    );
    assert_eq!(
        route_direct_browser_tool_name(&json!({
            "session_id": "browser-companion-1",
            "selector": "#submit"
        }))
        .expect("managed browser click routing should succeed"),
        "browser.companion.click"
    );
    assert_eq!(
        route_direct_browser_tool_name(&json!({
            "session_id": "browser-companion-1",
            "selector": "#query",
            "text": "hello"
        }))
        .expect("managed browser type routing should succeed"),
        "browser.companion.type"
    );
    assert_eq!(
        route_direct_browser_tool_name(&json!({
            "action": "stop",
            "session_id": "browser-companion-1"
        }))
        .expect("managed browser stop routing should succeed"),
        "browser.companion.session.stop"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_returns_coarse_fallback_for_zero_match_queries() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-coarse-fallback");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "orbit banana nebula",
                "limit": 4
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        !results.is_empty(),
        "coarse fallback should surface visible tools instead of an empty set"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["why"].as_array().is_some_and(|why| why
                .iter()
                .any(|reason| reason.as_str() == Some("coarse_fallback")))),
        "coarse fallback results should explain their fallback mode: {results:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_tool_search_returns_runtime_ready_companion_entries() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-search-browser-companion-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("request.json");
    let script_path = write_browser_companion_script(
        &root,
        "browser-companion-search",
        r#"{"ok":true,"result":{"ready":true}}"#,
        &log_path,
    );
    let config = browser_companion_runtime_config(&root, script_path.display().to_string());

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "browser companion session navigate click", "limit": 8}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().any(|entry| entry["tool_id"] == "browser"),
        "browser should absorb managed browser companion capabilities once runtime-ready: {results:?}"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["tool_id"] != "browser-session-start"),
        "managed browser primitives should stay collapsed beneath browser: {results:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_tools_split_read_and_write_execution_kinds() {
    let read =
        resolve_tool_execution("browser.companion.snapshot").expect("snapshot tool should resolve");
    assert_eq!(read.canonical_name, "browser.companion.snapshot");
    assert_eq!(read.execution_kind, ToolExecutionKind::Core);

    let write =
        resolve_tool_execution("browser.companion.click").expect("click tool should resolve");
    assert_eq!(write.canonical_name, "browser.companion.click");
    assert_eq!(write.execution_kind, ToolExecutionKind::App);
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_protocol_start_issues_managed_session_id_and_records_request() {
    let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
    let root = unique_tool_temp_dir("loongclaw-browser-companion-start");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("request.json");
    let script_path = write_browser_companion_script(
        &root,
        "browser-companion-ok",
        r#"{"ok":true,"result":{"page_url":"https://example.com","title":"Example Domain"}}"#,
        &log_path,
    );
    let config = browser_companion_runtime_config(&root, script_path.display().to_string());

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({
                "url": "https://example.com"
            }),
        },
        &config,
    )
    .expect("browser companion start should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["adapter"], "browser-companion");
    assert_eq!(
        outcome.payload["tool_name"],
        "browser.companion.session.start"
    );
    let session_id = outcome.payload["session_id"]
        .as_str()
        .expect("session id should be text");
    assert!(
        session_id.starts_with("browser-companion-"),
        "session id should be issued by LoongClaw: {session_id}"
    );
    assert_eq!(outcome.payload["result"]["page_url"], "https://example.com");

    let request: Value = serde_json::from_str(
        &std::fs::read_to_string(&log_path).expect("request log should exist"),
    )
    .expect("request log should be valid json");
    assert_eq!(request["tool_name"], "browser.companion.session.start");
    assert_eq!(request["operation"], "session.start");
    assert_eq!(request["action_class"], "read");
    assert_eq!(request["arguments"]["url"], "https://example.com");
    assert_eq!(request["session_id"], session_id);

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_protocol_rejects_unknown_session_for_read_tools() {
    let root = unique_tool_temp_dir("loongclaw-browser-companion-unknown-session");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("request.json");
    let script_path = write_browser_companion_script(
        &root,
        "browser-companion-unused",
        r#"{"ok":true,"result":{"page_url":"https://example.com"}}"#,
        &log_path,
    );
    let config = browser_companion_runtime_config(&root, script_path.display().to_string());

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.navigate".to_owned(),
            payload: json!({
                "session_id": "browser-companion-missing",
                "url": "https://example.com/next"
            }),
        },
        &config,
    )
    .expect_err("unknown companion session should fail closed");

    assert!(
        error.contains("browser_companion_unknown_session"),
        "error={error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_protocol_surfaces_invalid_json_from_command() {
    let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
    let root = unique_tool_temp_dir("loongclaw-browser-companion-invalid-json");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("request.json");
    let script_path = write_browser_companion_script(
        &root,
        "browser-companion-invalid-json",
        "not-json",
        &log_path,
    );
    let config = browser_companion_runtime_config(&root, script_path.display().to_string());

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({
                "url": "https://example.com"
            }),
        },
        &config,
    )
    .expect_err("invalid json should become a typed adapter failure");

    assert!(
        error.contains("browser_companion_protocol_invalid_json"),
        "error={error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_protocol_times_out_stalled_command() {
    let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
    let root = unique_tool_temp_dir("loong-browser-companion-timeout");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let script_path = write_browser_companion_sleep_script(&root, "browser-companion-timeout", 2);
    let mut config = browser_companion_runtime_config(&root, script_path.display().to_string());
    config.browser_companion.timeout_seconds = 1;

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({
                "url": "https://example.com"
            }),
        },
        &config,
    )
    .expect_err("hung command should time out");

    assert!(error.contains("browser_companion_timeout"), "error={error}");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-browser")]
#[test]
fn browser_companion_app_tool_click_uses_current_session_scope() {
    let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
    let root = unique_tool_temp_dir("loongclaw-browser-companion-app-click");
    std::fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("request.json");
    let script_path = write_browser_companion_script(
        &root,
        "browser-companion-app-click",
        r#"{"ok":true,"result":{"clicked":true}}"#,
        &log_path,
    );
    let runtime_config = browser_companion_runtime_config(&root, script_path.display().to_string());
    let start = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({
                "url": "https://example.com",
                BROWSER_SESSION_SCOPE_FIELD: "root-session"
            }),
        },
        &runtime_config,
    )
    .expect("browser companion start should succeed");
    let session_id = start.payload["session_id"]
        .as_str()
        .expect("session id should exist")
        .to_owned();

    let mut env = ScopedEnv::new();
    env.set("LOONG_BROWSER_COMPANION_READY", "true");

    let mut tool_config = crate::config::ToolConfig::default();
    tool_config.browser_companion.enabled = true;
    tool_config.browser_companion.command = Some(script_path.display().to_string());

    let outcome = execute_app_tool_with_config(
        ToolCoreRequest {
            tool_name: "browser.companion.click".to_owned(),
            payload: json!({
                "session_id": session_id,
                "selector": "#submit"
            }),
        },
        "root-session",
        &crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        &tool_config,
    )
    .expect("browser companion click should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["action_class"], "write");
    assert_eq!(outcome.payload["result"]["clicked"], true);

    let request: Value = serde_json::from_str(
        &std::fs::read_to_string(&log_path).expect("request log should exist"),
    )
    .expect("request log should be valid json");
    assert_eq!(request["operation"], "click");
    assert_eq!(request["action_class"], "write");
    assert_eq!(request["session_scope"], "root-session");
    assert_eq!(request["arguments"]["selector"], "#submit");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_reports_no_required_field_groups_for_bundled_skill_install() {
    let descriptor = catalog::tool_catalog()
        .descriptor("external_skills.install")
        .expect("external_skills.install should exist in the catalog");
    let searchable = searchable_entry_from_descriptor(descriptor);

    assert!(
        descriptor.required_fields().is_empty(),
        "schema-derived search should keep grouped requirements separate"
    );
    assert!(
        searchable.required_fields.is_empty(),
        "search should not flatten grouped alternatives into required_fields"
    );
    assert_eq!(searchable.required_field_groups, Vec::<Vec<String>>::new());
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn tool_search_respects_visible_tool_ids_from_runtime_context() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-search-visible-filter-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "session history status",
                "_loong": {
                    "tool_search": {
                        "visible_tool_ids": ["tool.search", "tool.invoke", "file.read"],
                    }
                }
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().all(|entry| !entry["tool_id"]
            .as_str()
            .is_some_and(|tool_id| tool_id.starts_with("session"))),
        "search should honor the injected visible tool surface: {results:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn runtime_discoverable_tool_entries_intersect_injected_view_with_runtime_surface() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.sessions_enabled = false;

    let injected = ToolView::from_tool_names(["sessions_list", "config.import"]);
    let names = runtime_discoverable_tool_entries(&config, Some(&injected))
        .into_iter()
        .map(|entry| entry.canonical_name)
        .collect::<Vec<_>>();

    assert!(
        names.contains(&"config.import".to_owned()),
        "expected enabled injected tool to remain visible: {names:?}"
    );
    assert!(
        !names.contains(&"sessions_list".to_owned()),
        "disabled runtime tool should not be re-exposed by injected visibility: {names:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn tool_search_rejects_forged_visible_tool_ids_from_untrusted_payload() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-search-visible-forged-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "session history status",
                "_loong": {
                    "tool_search": {
                        "visible_tool_ids": ["tool.search", "tool.invoke", "file.read"],
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("untrusted tool search should reject reserved internal visibility context");

    assert!(
        error.contains("payload._loong is reserved for trusted internal tool context"),
        "error={error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn web_fetch_respects_runtime_narrowing_from_trusted_internal_payload() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-web-fetch-runtime-narrowing-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let mut config = test_tool_runtime_config(root);
    config.web_fetch.timeout_seconds = 1;
    let error = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "web.fetch".to_owned(),
            payload: json!({
                "url": "https://example.com/docs",
                "_loong": {
                    "runtime_narrowing": {
                        "web_fetch": {
                            "allowed_domains": ["docs.example.com"],
                            "allow_private_hosts": false
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("runtime-narrowed child web.fetch should be denied before network access");

    assert!(
        error.contains("not in allowed_domains"),
        "expected narrowed domain denial, got: {error}"
    );
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn web_fetch_denies_disjoint_allowlists_when_runtime_narrowing_intersection_is_empty() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-web-fetch-runtime-narrowing-disjoint-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let mut config = test_tool_runtime_config(root.clone());
    config
        .web_fetch
        .allowed_domains
        .insert("api.example.com".to_owned());
    let error = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "web.fetch".to_owned(),
            payload: json!({
                "url": "https://api.example.com/docs",
                "_loong": {
                    "runtime_narrowing": {
                        "web_fetch": {
                            "allowed_domains": ["docs.example.com"]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("disjoint allowlists should deny domains allowed by only one side");

    assert!(
        error.contains("not in allowed_domains"),
        "expected empty-intersection allowlist denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn web_fetch_fail_closes_malformed_trusted_runtime_narrowing() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-web-fetch-runtime-narrowing-malformed-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "web.fetch".to_owned(),
            payload: json!({
                "url": "https://outside.invalid/docs",
                "_loong": {
                    "runtime_narrowing": "not-an-object"
                }
            }),
        },
        &config,
    )
    .expect_err("malformed trusted runtime narrowing should fail closed");

    assert!(
        error.contains("invalid_internal_runtime_narrowing"),
        "expected parse failure, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn web_fetch_rejects_forged_runtime_narrowing_from_untrusted_payload() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-web-fetch-runtime-narrowing-forged-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "web.fetch".to_owned(),
            payload: json!({
                "url": "https://example.com/docs",
                "_loong": {
                    "runtime_narrowing": {
                        "web_fetch": {
                            "allowed_domains": ["docs.example.com"]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("untrusted runtime narrowing should be rejected");

    assert!(
        error.contains("payload._loong is reserved for trusted internal tool context"),
        "error={error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "feishu-integration")]
#[test]
fn tool_search_includes_feishu_tools_when_runtime_configured() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
        channel: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
            app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
            ..crate::config::FeishuChannelConfig::default()
        },
        integration: crate::config::FeishuIntegrationConfig::default(),
    });

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "feishu message search", "limit": 8}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().any(|entry| entry["tool_id"] == "channel"),
        "feishu runtime tools should collapse beneath the channel surface: {results:?}"
    );
    assert!(
        results
            .iter()
            .all(|entry| entry["tool_id"] != "feishu-messages-send"),
        "feishu-specific ids should stay hidden behind the channel surface: {results:?}"
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_searchable_entries_report_anyof_required_groups() {
    let entry = feishu_searchable_entries()
        .into_iter()
        .find(|entry| entry.canonical_name == "feishu.doc.append")
        .expect("feishu.doc.append should be discoverable");

    assert_eq!(entry.required_fields, vec!["url".to_owned()]);
    assert_eq!(
        entry.required_field_groups,
        vec![
            vec!["url".to_owned(), "content".to_owned()],
            vec!["url".to_owned(), "content_path".to_owned()],
        ]
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn tool_invoke_dispatches_feishu_discovered_tool_with_a_valid_lease() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("tool-invoke-feishu-send");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-tool-invoke-feishu-send",
        &["offline_access"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "send feishu message", "limit": 8}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let result = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "channel")
        .expect("channel search result");

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "channel",
                "lease": result["lease"].clone(),
                "arguments": {
                    "operation": "messages.send",
                    "text": "ship by invoke"
                }
            }),
        },
        &config,
    )
    .expect_err("missing receive_id should fail after discovery-first invoke routing");

    assert!(
        error.contains("feishu.messages.send requires payload.receive_id"),
        "error={error}"
    );

    fs::remove_dir_all(&temp_dir).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_invoke_dispatches_a_discovered_tool_with_a_valid_lease() {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-tool-invoke-{nanos}"));
    fs::create_dir_all(&root).expect("create fixture root");
    fs::write(root.join("README.md"), "tool invoke fixture").expect("write fixture");

    let config = test_tool_runtime_config(root.clone());
    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "external skills policy"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let result = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "skills")
        .expect("skills search result");

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "skills",
                "lease": result["lease"].clone(),
                "arguments": {
                    "operation": "policy",
                    "action": "get"
                }
            }),
        },
        &config,
    )
    .expect("tool invoke should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["action"], "get");
    assert!(outcome.payload["policy"].is_object());

    fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn discovered_tool_lease_uses_current_catalog_digest() {
    let root = unique_tool_temp_dir("loongclaw-tool-lease-digest");
    let config = test_tool_runtime_config(root.clone());
    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "switch provider"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let lease = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "agent")
        .and_then(|entry| entry["lease"].as_str())
        .expect("agent lease");
    let lease_parts = lease.split_once('.').expect("lease separator");
    let encoded_claims = lease_parts.0;
    let claims_bytes = URL_SAFE_NO_PAD
        .decode(encoded_claims)
        .expect("decode claims");
    let claims: Value = serde_json::from_slice(&claims_bytes).expect("parse claims");
    let expected_digest = tool_lease_authority::tool_catalog_digest();
    let repeated_digest = tool_lease_authority::tool_catalog_digest();

    assert_eq!(claims["catalog_digest"], json!(expected_digest));
    assert_eq!(repeated_digest, expected_digest);

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_invoke_rejects_tampered_or_missing_leases() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-invoke-invalid-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "file.read",
                "lease": "tampered",
                "arguments": {
                    "path": "README.md"
                }
            }),
        },
        &config,
    )
    .expect_err("tampered lease should fail");

    assert!(error.contains("invalid_tool_lease"), "error: {error}");
    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_invoke_rejects_leases_replayed_in_another_turn() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-invoke-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "external skills policy",
                TOOL_LEASE_SESSION_ID_FIELD: "session-a",
                TOOL_LEASE_TURN_ID_FIELD: "turn-a"
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let result = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "skills")
        .expect("skills search result");

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "skills",
                "lease": result["lease"].clone(),
                "arguments": {
                    "operation": "policy",
                    "action": "get"
                },
                TOOL_LEASE_SESSION_ID_FIELD: "session-a",
                TOOL_LEASE_TURN_ID_FIELD: "turn-b"
            }),
        },
        &config,
    )
    .expect_err("replayed turn lease should fail");

    assert!(error.contains("turn mismatch"), "error: {error}");
    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-webfetch")]
#[test]
fn tool_invoke_preserves_trusted_runtime_narrowing_for_inner_execution() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-invoke-runtime-narrowing-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let mut config = test_tool_runtime_config(root.clone());
    config
        .web_fetch
        .allowed_domains
        .insert("outside.invalid".to_owned());

    let (tool_name, mut payload) = bridge_provider_tool_call_with_scope(
        "web.fetch",
        json!({
            "url": "https://outside.invalid/docs"
        }),
        None,
        None,
    );
    let payload_object = payload.as_object_mut().expect("tool.invoke payload object");
    payload_object.insert(
        LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        json!({
            LOONG_INTERNAL_RUNTIME_NARROWING_KEY: {
                "web_fetch": {
                    "allowed_domains": ["docs.example.com"]
                }
            }
        }),
    );

    let error =
        execute_tool_core_with_test_context(ToolCoreRequest { tool_name, payload }, &config)
            .expect_err("tool.invoke should preserve trusted narrowing for inner web.fetch");

    assert!(
        error.contains("not in allowed_domains"),
        "expected inner web.fetch denial after narrowing, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn tool_invoke_rejects_forged_reserved_internal_context_inside_arguments() {
    let root = std::env::temp_dir().join(format!(
        "loongclaw-tool-invoke-inner-context-forged-{}",
        std::process::id()
    ));
    let fixture_path = root.join("README.md");

    std::fs::create_dir_all(&root).expect("create fixture root");
    std::fs::write(&fixture_path, "tool invoke fixture").expect("write fixture file");

    let config = test_tool_runtime_config(root.clone());
    let (tool_name, mut payload) = bridge_provider_tool_call_with_scope(
        "file.read",
        json!({
            "path": fixture_path.display().to_string()
        }),
        None,
        None,
    );
    let payload_object = payload.as_object_mut().expect("tool.invoke payload object");
    let arguments = payload_object
        .get_mut("arguments")
        .and_then(Value::as_object_mut)
        .expect("tool.invoke arguments object");
    arguments.insert(
        LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        json!({
            LOONG_INTERNAL_TOOL_SEARCH_KEY: {
                LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY: ["file.read"]
            }
        }),
    );

    let error = execute_tool_core_with_config(ToolCoreRequest { tool_name, payload }, &config)
        .expect_err("untrusted tool.invoke should reject forged inner reserved context");

    assert!(
        error.contains("payload.arguments._loong is reserved for trusted internal tool context"),
        "error={error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn tool_search_hides_tools_exceeding_granted_capabilities() {
    let created_at = std::time::SystemTime::now();
    let duration = created_at
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after epoch");
    let nanos = duration.as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-tool-search-cap-filter-{nanos}"));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let result = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "read",
                TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD: [
                    "InvokeTool"
                ]
            }),
        },
        &config,
    )
    .expect("search should succeed");

    let results = result.payload["results"].as_array().expect("results array");
    let tool_ids: Vec<&str> = results
        .iter()
        .filter_map(|entry| entry["tool_id"].as_str())
        .collect();
    assert!(
        !tool_ids.contains(&"file.read"),
        "file.read requires FilesystemRead, should be hidden when only InvokeTool is granted; got: {tool_ids:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_hides_bash_exec_without_side_effect_capabilities() {
    let created_at = std::time::SystemTime::now();
    let duration = created_at
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after epoch");
    let nanos = duration.as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-tool-search-bash-cap-filter-{nanos}"));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let mut config = test_tool_runtime_config(root.clone());
    config.bash_exec = ready_bash_exec_runtime_policy();

    let result = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "bash.exec",
                TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD: [
                    "InvokeTool"
                ]
            }),
        },
        &config,
    )
    .expect("search should succeed");

    let results = result.payload["results"].as_array().expect("results array");
    assert!(
        results.iter().all(|entry| entry["tool_id"] != "bash.exec"),
        "bash.exec should be hidden when the granted capabilities only include InvokeTool; got: {results:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn is_known_tool_name_accepts_canonical_and_alias_forms() {
    assert!(is_known_tool_name("config.import"));
    assert!(is_known_tool_name("config_import"));
    assert!(is_known_tool_name("claw.migrate"));
    assert!(is_known_tool_name("claw_migrate"));
    assert!(is_known_tool_name("external_skills.policy"));
    assert!(is_known_tool_name("external_skills_policy"));
    assert!(is_known_tool_name("external_skills.fetch"));
    assert!(is_known_tool_name("external_skills_fetch"));
    assert!(is_known_tool_name("file.read"));
    assert!(is_known_tool_name("file_read"));
    assert!(is_known_tool_name("file.write"));
    assert!(is_known_tool_name("file_write"));
    assert!(is_known_tool_name("provider.switch"));
    assert!(is_known_tool_name("provider_switch"));
    assert!(is_known_tool_name("browser.open"));
    assert!(is_known_tool_name("browser_open"));
    assert!(is_known_tool_name("browser.extract"));
    assert!(is_known_tool_name("browser_extract"));
    assert!(is_known_tool_name("browser.click"));
    assert!(is_known_tool_name("browser_click"));
    assert!(is_known_tool_name("shell.exec"));
    assert!(is_known_tool_name("shell_exec"));
    assert!(is_known_tool_name("shell"));
    #[cfg(feature = "tool-http")]
    {
        assert!(is_known_tool_name(HTTP_REQUEST_TOOL_NAME));
        assert!(is_known_tool_name("http_request"));
    }
    #[cfg(not(feature = "tool-http"))]
    {
        assert!(!is_known_tool_name(HTTP_REQUEST_TOOL_NAME));
        assert!(!is_known_tool_name("http_request"));
    }
    assert!(is_known_tool_name("web.fetch"));
    assert!(is_known_tool_name("web_fetch"));
    assert!(is_known_tool_name("feishu.whoami"));
    assert!(is_known_tool_name("feishu_whoami"));
    assert!(is_known_tool_name("feishu.doc.create"));
    assert!(is_known_tool_name("feishu_doc_create"));
    assert!(is_known_tool_name("feishu.doc.append"));
    assert!(is_known_tool_name("feishu_doc_append"));
    assert!(is_known_tool_name("feishu.doc.read"));
    assert!(is_known_tool_name("feishu_doc_read"));
    assert!(is_known_tool_name("feishu.messages.history"));
    assert!(is_known_tool_name("feishu_messages_history"));
    assert!(is_known_tool_name("feishu.messages.get"));
    assert!(is_known_tool_name("feishu_messages_get"));
    assert!(is_known_tool_name("feishu.messages.resource.get"));
    assert!(is_known_tool_name("feishu_messages_resource_get"));
    assert!(is_known_tool_name("feishu.messages.search"));
    assert!(is_known_tool_name("feishu_messages_search"));
    assert!(is_known_tool_name("feishu.messages.send"));
    assert!(is_known_tool_name("feishu_messages_send"));
    assert!(is_known_tool_name("feishu.messages.reply"));
    assert!(is_known_tool_name("feishu_messages_reply"));
    assert!(is_known_tool_name("feishu.card.update"));
    assert!(is_known_tool_name("feishu_card_update"));
    assert!(is_known_tool_name("feishu.calendar.list"));
    assert!(is_known_tool_name("feishu_calendar_list"));
    assert!(is_known_tool_name("feishu.calendar.freebusy"));
    assert!(is_known_tool_name("feishu_calendar_freebusy"));
    assert!(is_known_tool_name("agent"));
    assert!(is_known_tool_name("skills"));
    assert!(!is_known_tool_name("nonexistent.tool"));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn tool_registry_with_config_includes_feishu_tools_when_runtime_configured() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
        channel: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
            app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
            ..crate::config::FeishuChannelConfig::default()
        },
        integration: crate::config::FeishuIntegrationConfig::default(),
    });

    let entries = tool_registry_with_config(Some(&config));
    let names = entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();

    assert!(names.contains(&"feishu.whoami".to_owned()));
    assert!(names.contains(&"feishu.doc.create".to_owned()));
    assert!(names.contains(&"feishu.doc.append".to_owned()));
    assert!(names.contains(&"feishu.doc.read".to_owned()));
    assert!(names.contains(&"feishu.messages.history".to_owned()));
    assert!(names.contains(&"feishu.messages.get".to_owned()));
    assert!(names.contains(&"feishu.messages.resource.get".to_owned()));
    assert!(names.contains(&"feishu.messages.search".to_owned()));
    assert!(names.contains(&"feishu.messages.send".to_owned()));
    assert!(names.contains(&"feishu.messages.reply".to_owned()));
    assert!(names.contains(&"feishu.card.update".to_owned()));
    assert!(names.contains(&"feishu.calendar.list".to_owned()));
    assert!(names.contains(&"feishu.calendar.freebusy".to_owned()));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_keeps_direct_surface_when_feishu_runtime_is_configured() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
        channel: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
            app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
            ..crate::config::FeishuChannelConfig::default()
        },
        integration: crate::config::FeishuIntegrationConfig::default(),
    });

    let defs = provider_tool_definitions_with_config(Some(&config));
    let names = defs
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert!(names.contains(&"browser"));
    assert!(names.contains(&"exec"));
    assert!(names.contains(&"read"));
    assert!(names.contains(&"tool_invoke"));
    assert!(names.contains(&"tool_search"));
    assert!(names.contains(&"web"));
    assert!(names.contains(&"write"));
    assert_eq!(names.len(), 7);
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_tool_metadata_catalog_is_self_consistent() {
    let aliases = feishu::feishu_tool_alias_pairs();
    let registry = feishu::feishu_tool_registry_entries();
    let defs = feishu::feishu_provider_tool_definitions();

    let registry_names = registry
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<BTreeSet<_>>();
    let definition_names = defs
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .map(canonical_tool_name)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    assert_eq!(registry_names, definition_names);
    for (alias, canonical) in aliases {
        assert_eq!(canonical_tool_name(alias), *canonical);
        assert!(feishu::is_known_feishu_tool_name(alias));
        assert!(feishu::is_known_feishu_tool_name(canonical));
    }
    for name in registry_names {
        assert!(feishu::is_known_feishu_tool_name(&name));
    }
}

#[cfg(all(feature = "tool-file", feature = "tool-websearch"))]
#[test]
fn tool_search_creates_tool_lease_secret_under_scoped_runtime_home() {
    let root = unique_tool_temp_dir("loongclaw-tool-search-home-override");
    let memory_dir = root.join("memory");

    std::fs::create_dir_all(&memory_dir).expect("create memory dir");

    let config = test_tool_runtime_config(root);
    let expected_home = config._runtime_home.path().to_path_buf();
    let expected_secret = expected_home.join("tool-lease-secret.hex");
    let payload = json!({
        "query": "edit file",
        "limit": 8
    });
    let request = ToolCoreRequest {
        tool_name: "tool.search".to_owned(),
        payload,
    };
    let outcome =
        execute_tool_core_with_config(request, &config).expect("tool search should succeed");

    assert_eq!(outcome.status, "ok");
    assert!(expected_secret.exists());
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_shape_examples_reference_only_known_feishu_tools() {
    let shapes = feishu::feishu_shape_examples();
    let names = shapes.keys().copied().collect::<BTreeSet<_>>();

    assert!(names.contains("feishu.whoami"));
    assert!(names.contains("feishu.doc.create"));
    assert!(names.contains("feishu.doc.append"));
    assert!(names.contains("feishu.doc.read"));
    assert!(names.contains("feishu.messages.get"));
    assert!(names.contains("feishu.messages.history"));
    assert!(names.contains("feishu.messages.search"));
    assert!(names.contains("feishu.card.update"));
    assert!(names.contains("feishu.calendar.list"));
    assert!(names.contains("feishu.calendar.freebusy"));
    #[cfg(feature = "tool-file")]
    assert!(names.contains("feishu.messages.resource.get"));

    for name in names {
        assert!(feishu::is_known_feishu_tool_name(name));
        assert!(shapes.get(name).and_then(Value::as_object).is_some());
    }
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_message_write_uuid() {
    let defs = feishu::feishu_provider_tool_definitions();
    let send = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_send")
        .expect("send definition should exist");
    let reply = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_reply")
        .expect("reply definition should exist");

    assert_eq!(
        send["function"]["parameters"]["properties"]["uuid"]["type"],
        "string"
    );
    assert_eq!(
        reply["function"]["parameters"]["properties"]["uuid"]["type"],
        "string"
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_message_post_and_media_payloads() {
    let defs = feishu::feishu_provider_tool_definitions();
    let send = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_send")
        .expect("send definition should exist");
    let reply = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_reply")
        .expect("reply definition should exist");
    let card_update = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_card_update")
        .expect("card update definition should exist");

    assert!(
        send["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("post")
                && description.contains("image")
                && description.contains("file"))
    );
    assert!(
        reply["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("post")
                && description.contains("image")
                && description.contains("file"))
    );
    assert_eq!(
        send["function"]["parameters"]["properties"]["post"]["type"],
        "object"
    );
    assert_eq!(
        reply["function"]["parameters"]["properties"]["post"]["type"],
        "object"
    );
    assert_eq!(
        send["function"]["parameters"]["properties"]["image_key"]["type"],
        "string"
    );
    assert_eq!(
        send["function"]["parameters"]["properties"]["file_key"]["type"],
        "string"
    );
    assert_eq!(
        reply["function"]["parameters"]["properties"]["image_key"]["type"],
        "string"
    );
    assert_eq!(
        reply["function"]["parameters"]["properties"]["file_key"]["type"],
        "string"
    );
    #[cfg(feature = "tool-file")]
    {
        assert_eq!(
            send["function"]["parameters"]["properties"]["image_path"]["type"],
            "string"
        );
        assert_eq!(
            send["function"]["parameters"]["properties"]["file_path"]["type"],
            "string"
        );
        assert_eq!(
            send["function"]["parameters"]["properties"]["file_type"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["image_path"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["file_path"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["file_type"]["type"],
            "string"
        );
    }
    assert_eq!(send["function"]["parameters"]["required"], json!([]));
    assert_eq!(reply["function"]["parameters"]["required"], json!([]));
    assert_eq!(
        card_update["function"]["parameters"]["properties"]["card"]["type"],
        "object"
    );
    assert_eq!(
        card_update["function"]["parameters"]["properties"]["markdown"]["type"],
        "string"
    );
    assert_eq!(card_update["function"]["parameters"]["required"], json!([]));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_ingress_defaults() {
    let defs = feishu::feishu_provider_tool_definitions();
    let send = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_send")
        .expect("send definition should exist");
    let doc_create = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_doc_create")
        .expect("doc create definition should exist");
    let doc_append = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_doc_append")
        .expect("doc append definition should exist");
    let reply = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_reply")
        .expect("reply definition should exist");
    let get = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_get")
        .expect("get definition should exist");
    let resource_get = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_resource_get")
        .expect("resource get definition should exist");
    let history = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_history")
        .expect("history definition should exist");

    assert!(
        send["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu conversation"))
    );
    assert!(
        doc_create["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("markdown or html"))
    );
    assert!(
        doc_append["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("existing Feishu document"))
    );
    assert_eq!(
        doc_create["function"]["parameters"]["properties"]["content_type"]["type"],
        "string"
    );
    assert_eq!(
        doc_create["function"]["parameters"]["properties"]["content_type"]["enum"],
        json!(["markdown", "html"])
    );
    assert_eq!(
        doc_create["function"]["parameters"]["properties"]["content"]["type"],
        "string"
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        doc_create["function"]["parameters"]["properties"]["content_path"]["type"],
        "string"
    );
    assert_eq!(
        doc_append["function"]["parameters"]["properties"]["url"]["type"],
        "string"
    );
    assert_eq!(
        doc_append["function"]["parameters"]["properties"]["content"]["type"],
        "string"
    );
    assert_eq!(
        doc_append["function"]["parameters"]["properties"]["content_type"]["enum"],
        json!(["markdown", "html"])
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        doc_append["function"]["parameters"]["properties"]["content_path"]["type"],
        "string"
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        doc_append["function"]["parameters"]["required"],
        json!(["url"])
    );
    #[cfg(feature = "tool-file")]
    assert_eq!(
        doc_append["function"]["parameters"]["anyOf"],
        json!([
            { "required": ["content"] },
            { "required": ["content_path"] }
        ])
    );
    #[cfg(not(feature = "tool-file"))]
    assert_eq!(
        doc_append["function"]["parameters"]["required"],
        json!(["url", "content"])
    );
    assert!(
        reply["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("source Feishu message"))
    );
    assert!(
        get["function"]["parameters"]["properties"]["message_id"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu ingress"))
    );
    assert!(
        resource_get["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("download")
                && description.contains("configured file root"))
    );
    assert!(
        resource_get["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("resource_inventory")
                && description.contains("uniquely identifies")
                && description.contains(
                    "payload.message_id is omitted or matches the current ingress message"
                )
                && description.contains("Outside the current ingress turn")
                && description.contains("source message_id"))
    );
    assert!(
        resource_get["function"]["parameters"]["properties"]["message_id"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu ingress")
                && description.contains("Outside the current ingress turn"))
    );
    assert!(
        resource_get["function"]["parameters"]["properties"]["file_key"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("exactly one Feishu message resource")
                    && description.contains("payload.type uniquely selects")
                    && description.contains("same message")
                    && description.contains("resource_inventory")
                    && description.contains("multiple resources")
            })
    );
    assert_eq!(
        resource_get["function"]["parameters"]["properties"]["file_key"]["type"],
        "string"
    );
    assert_eq!(
        resource_get["function"]["parameters"]["properties"]["save_as"]["type"],
        "string"
    );
    assert_eq!(
        resource_get["function"]["parameters"]["properties"]["type"]["enum"],
        json!(["image", "file", "audio", "media"])
    );
    assert!(
        resource_get["function"]["parameters"]["properties"]["type"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("audio")
                    && description.contains("media")
                    && description.contains("normalize")
                    && description.contains("preview")
                    && description.contains("resource_inventory")
                    && description.contains("exactly one Feishu message resource")
                    && description.contains("same message")
                    && description.contains("payload.file_key uniquely selects")
            })
    );
    assert_eq!(
        resource_get["function"]["parameters"]["required"],
        json!(["save_as"])
    );
    assert!(
        history["function"]["parameters"]["properties"]["container_id_type"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu conversation"))
    );
    assert_eq!(get["function"]["parameters"]["required"], json!([]));
    assert_eq!(history["function"]["parameters"]["required"], json!([]));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn feishu_shape_examples_advertise_explicit_resource_inventory_selection() {
    let shapes = feishu::feishu_shape_examples();
    assert_eq!(
        shapes.get("feishu.messages.resource.get"),
        Some(&json!({
            "message_id": "om_123",
            "file_key": "img_from_resource_inventory",
            "type": "image",
            "save_as": "downloads/preview.png"
        }))
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_card_update_shared_semantics() {
    let defs = feishu::feishu_provider_tool_definitions();
    let card_update = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_card_update")
        .expect("card update definition should exist");

    assert!(
        card_update["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("shared=true"))
    );
    assert_eq!(
        card_update["function"]["parameters"]["properties"]["shared"]["type"],
        "boolean"
    );
    assert!(
        card_update["function"]["parameters"]["properties"]["open_ids"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("shared=true"))
    );

    let shapes = feishu::feishu_shape_examples();
    assert_eq!(
        shapes.get("feishu.card.update"),
        Some(&json!({
            "shared": true,
            "markdown": "Approved for everyone"
        }))
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_card_update_markdown_shortcut() {
    let defs = feishu::feishu_provider_tool_definitions();
    let card_update = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_card_update")
        .expect("card update definition should exist");

    assert!(
        card_update["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("markdown"))
    );
    assert!(
        card_update["function"]["parameters"]["properties"]["markdown"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("standard markdown card")
                    && description.contains("Mutually exclusive with `card`")
            })
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_card_update_token_budget() {
    let defs = feishu::feishu_provider_tool_definitions();
    let card_update = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_card_update")
        .expect("card update definition should exist");

    assert!(
        card_update["function"]["description"]
            .as_str()
            .is_some_and(
                |description| description.contains("30 minutes") && description.contains("twice")
            )
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_advertises_feishu_search_ingress_scope() {
    let defs = feishu::feishu_provider_tool_definitions();
    let search = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_messages_search")
        .expect("search definition should exist");

    assert!(
        search["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu conversation"))
    );
    assert!(
        search["function"]["parameters"]["properties"]["chat_ids"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("current Feishu conversation"))
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_exposes_bitable_search_sort_as_array() {
    let defs = feishu::feishu_provider_tool_definitions();
    let search = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_bitable_record_search")
        .expect("bitable search definition should exist");

    assert_eq!(
        search["function"]["parameters"]["properties"]["sort"]["type"],
        "array"
    );
    assert_eq!(
        search["function"]["parameters"]["properties"]["sort"]["items"]["type"],
        "object"
    );
    assert_eq!(
        search["function"]["parameters"]["properties"]["automatic_fields"]["type"],
        "boolean"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_bitable_batch_record_tools_reject_more_than_500_items_before_network() {
    let temp_dir = unique_feishu_tool_temp_dir("bitable-batch-limit");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-batch-limit",
        &["offline_access", "base:record:write"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let create_records = (0..501)
        .map(|index| json!({ "fields": { "Name": format!("row-{index}") } }))
        .collect::<Vec<_>>();
    let create_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.bitable.record.batch_create".to_owned(),
            payload: json!({
                "app_token": "app_demo",
                "table_id": "tbl_demo",
                "records": create_records,
            }),
        },
        &config,
    )
    .expect_err("tool should reject >500 batch create items");
    assert!(
        create_error.contains("batch size must be <= 500"),
        "error={create_error}"
    );

    let update_records = (0..501)
            .map(|index| {
                json!({ "record_id": format!("rec_{index}"), "fields": { "Name": format!("row-{index}") } })
            })
            .collect::<Vec<_>>();
    let update_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.bitable.record.batch_update".to_owned(),
            payload: json!({
                "app_token": "app_demo",
                "table_id": "tbl_demo",
                "records": update_records,
            }),
        },
        &config,
    )
    .expect_err("tool should reject >500 batch update items");
    assert!(
        update_error.contains("batch size must be <= 500"),
        "error={update_error}"
    );

    let delete_records = (0..501)
        .map(|index| format!("rec_{index}"))
        .collect::<Vec<_>>();
    let delete_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.bitable.record.batch_delete".to_owned(),
            payload: json!({
                "app_token": "app_demo",
                "table_id": "tbl_demo",
                "records": delete_records,
            }),
        },
        &config,
    )
    .expect_err("tool should reject >500 batch delete items");
    assert!(
        delete_error.contains("batch size must be <= 500"),
        "error={delete_error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_bitable_field_tools_require_positive_type() {
    let temp_dir = unique_feishu_tool_temp_dir("bitable-field-type");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-field-type",
        &["offline_access", "bitable:app"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let create_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.bitable.field.create".to_owned(),
            payload: json!({
                "app_token": "app_demo",
                "table_id": "tbl_demo",
                "field_name": "Amount",
                "type": 0
            }),
        },
        &config,
    )
    .expect_err("field create should reject non-positive type");
    assert!(
        create_error.contains("feishu.bitable.field.create invalid payload.type"),
        "error={create_error}"
    );

    let update_error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.bitable.field.update".to_owned(),
            payload: json!({
                "app_token": "app_demo",
                "table_id": "tbl_demo",
                "field_id": "fld_demo",
                "field_name": "Amount",
                "type": 0
            }),
        },
        &config,
    )
    .expect_err("field update should reject non-positive type");
    assert!(
        update_error.contains("feishu.bitable.field.update invalid payload.type"),
        "error={update_error}"
    );
}

#[cfg(feature = "feishu-integration")]
#[test]
fn provider_tool_definitions_with_config_caps_bitable_list_page_size_at_100() {
    let defs = feishu::feishu_provider_tool_definitions();
    let list = defs
        .iter()
        .find(|item| item["function"]["name"] == "feishu_bitable_list")
        .expect("bitable list definition should exist");

    assert_eq!(
        list["function"]["parameters"]["properties"]["page_size"]["maximum"],
        100
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuToolMockRequest {
    method: String,
    path: String,
    query: Option<String>,
    authorization: Option<String>,
    body: String,
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[derive(Clone, Default)]
struct FeishuToolMockServerState {
    requests: std::sync::Arc<tokio::sync::Mutex<Vec<FeishuToolMockRequest>>>,
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
async fn record_feishu_tool_request(
    axum::extract::State(state): axum::extract::State<FeishuToolMockServerState>,
    request: axum::extract::Request,
) {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("read mock request body");
    state.requests.lock().await.push(FeishuToolMockRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(ToOwned::to_owned),
        authorization: parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        body: String::from_utf8(body.to_vec()).expect("mock request body utf8"),
    });
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
async fn spawn_feishu_tool_mock_server(
    router: axum::Router,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock feishu server");
    let address = listener.local_addr().expect("mock server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve mock feishu api");
    });
    (format!("http://{address}"), handle)
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
fn unique_feishu_tool_temp_dir(label: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    std::env::temp_dir().join(format!(
        "loongclaw-tool-feishu-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
fn seed_feishu_tool_grant(
    sqlite_path: &std::path::Path,
    access_token: &str,
    scopes: &[&str],
) -> crate::channel::feishu::api::FeishuTokenStore {
    seed_feishu_tool_grant_for_account(sqlite_path, "feishu_main", "ou_123", access_token, scopes)
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
fn seed_feishu_tool_grant_for_account(
    sqlite_path: &std::path::Path,
    account_id: &str,
    open_id: &str,
    access_token: &str,
    scopes: &[&str],
) -> crate::channel::feishu::api::FeishuTokenStore {
    let now_s = crate::channel::feishu::api::unix_ts_now();
    let store = crate::channel::feishu::api::FeishuTokenStore::new(sqlite_path.to_path_buf());
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: account_id.to_owned(),
                open_id: open_id.to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: access_token.to_owned(),
            refresh_token: format!("r-{access_token}"),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes(
                scopes.iter().copied(),
            ),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        })
        .expect("save grant");
    store
        .set_selected_grant(account_id, open_id, now_s + 1)
        .expect("select grant");
    store
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
fn build_feishu_tool_runtime_config(
    base_url: String,
    sqlite_path: &std::path::Path,
) -> runtime_config::ToolRuntimeConfig {
    runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                account_id: Some("feishu_main".to_owned()),
                app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
                app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
                base_url: Some(base_url),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    }
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_read_tool_uses_selected_grant_and_user_token() {
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        routing::get,
    };
    use tokio::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        authorization: Option<String>,
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    async fn record_request(State(state): State<MockServerState>, request: Request) {
        let (parts, body) = request.into_parts();
        let _ = to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        state.requests.lock().await.push(MockRequest {
            path: parts.uri.path().to_owned(),
            authorization: parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
        });
    }

    async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock feishu server");
        let address = listener.local_addr().expect("mock server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock feishu api");
        });
        (format!("http://{address}"), handle)
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-tool-feishu-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    let temp_dir = unique_temp_dir("doc-read");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/docx/v1/documents/doxcnDemo/raw_content",
        get({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "content": "hello from docs"
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let now_s = crate::channel::feishu::api::unix_ts_now();
    let store = crate::channel::feishu::api::FeishuTokenStore::new(sqlite_path.clone());
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-doc".to_owned(),
            refresh_token: "r-token-doc".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
                "docx:document:readonly",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        })
        .expect("save grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select grant");

    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                account_id: Some("feishu_main".to_owned()),
                app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
                app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
                base_url: Some(base_url),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.read".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnDemo"
            }),
        },
        &config,
    )
    .expect("feishu doc read tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["document"]["document_id"], "doxcnDemo");
    assert_eq!(outcome.payload["document"]["content"], "hello from docs");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-doc")
    );
    assert_eq!(
        requests[0].path,
        "/open-apis/docx/v1/documents/doxcnDemo/raw_content"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_create_tool_uses_selected_grant_and_user_token() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-create");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "document": {
                                    "document_id": "doxcnCreated",
                                    "revision_id": 1,
                                    "title": "Release Plan"
                                }
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-heading"],
                                "blocks": [
                                    {
                                        "block_id": "tmp-heading",
                                        "block_type": 3,
                                        "heading1": {
                                            "elements": [{"text_run": {"content": "Release Plan"}}]
                                        },
                                        "children": []
                                    }
                                ]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [
                                    {
                                        "block_id": "doxcnRealHeading",
                                        "temporary_block_id": "tmp-heading"
                                    }
                                ]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-create",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.create".to_owned(),
            payload: serde_json::json!({
                "title": "Release Plan",
                "content": "# Release Plan",
                "content_type": "markdown"
            }),
        },
        &config,
    )
    .expect("feishu doc create tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["document"]["document_id"], "doxcnCreated");
    assert_eq!(outcome.payload["document"]["title"], "Release Plan");
    assert_eq!(outcome.payload["document"]["revision_id"], 1);
    assert_eq!(
        outcome.payload["document"]["url"],
        "https://open.feishu.cn/docx/doxcnCreated"
    );
    assert_eq!(outcome.payload["content_inserted"], true);
    assert_eq!(outcome.payload["inserted_block_count"], 1);
    assert_eq!(outcome.payload["insert_batch_count"], 1);
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.authorization.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("Bearer u-token-doc-create"),
            Some("Bearer u-token-doc-create"),
            Some("Bearer u-token-doc-create"),
        ]
    );
    assert_eq!(requests[0].path, "/open-apis/docx/v1/documents");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).expect("create request json"),
        serde_json::json!({
            "title": "Release Plan"
        })
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/blocks/convert"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).expect("convert request json"),
        serde_json::json!({
            "content_type": "markdown",
            "content": "# Release Plan"
        })
    );
    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant"
    );
    assert_eq!(
        requests[2].query.as_deref(),
        Some("document_revision_id=-1")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[2].body).expect("descendant request json"),
        serde_json::json!({
            "children_id": ["tmp-heading"],
            "descendants": [
                {
                    "block_id": "tmp-heading",
                    "block_type": 3,
                    "heading1": {
                        "elements": [{"text_run": {"content": "Release Plan"}}]
                    },
                    "children": []
                }
            ],
            "index": -1
        })
    );

    server.abort();
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_create_tool_reads_content_path_under_safe_file_root() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-create-content-path");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("content-root");
    let content_path = file_root.join("docs/release-plan.md");
    fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    fs::write(&content_path, "# Release Plan").expect("write content fixture");

    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "document": {
                                    "document_id": "doxcnCreated",
                                    "revision_id": 1,
                                    "title": "Release Plan",
                                    "url": "https://open.feishu.cn/docx/doxcnCreated"
                                }
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-heading"],
                                "blocks": [
                                    {
                                        "block_id": "tmp-heading",
                                        "block_type": 3,
                                        "heading1": {
                                            "elements": [{"text_run": {"content": "Release Plan"}}]
                                        },
                                        "children": []
                                    }
                                ]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [
                                    {
                                        "block_id": "doxcnRealHeading",
                                        "temporary_block_id": "tmp-heading"
                                    }
                                ]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-create-path",
        &["offline_access", "docx:document"],
    );
    let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
    config.file_root = Some(file_root.clone());

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.create".to_owned(),
            payload: serde_json::json!({
                "title": "Release Plan",
                "content_path": "docs/release-plan.md"
            }),
        },
        &config,
    )
    .expect("feishu doc create tool should read content_path");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content_inserted"], true);
    assert_eq!(outcome.payload["content_type"], "markdown");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).expect("convert request json"),
        serde_json::json!({
            "content_type": "markdown",
            "content": "# Release Plan"
        })
    );

    server.abort();
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[test]
fn feishu_doc_create_tool_rejects_content_and_content_path_together() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("doc-create-content-conflict");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("content-root");
    let content_path = file_root.join("docs/release-plan.md");
    fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    fs::write(&content_path, "# Release Plan").expect("write content fixture");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-create-conflict",
        &["offline_access", "docx:document"],
    );
    let mut config =
        build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
    config.file_root = Some(file_root);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.create".to_owned(),
            payload: serde_json::json!({
                "title": "Release Plan",
                "content": "# Inline",
                "content_path": "docs/release-plan.md"
            }),
        },
        &config,
    )
    .expect_err("doc create should reject inline content mixed with content_path");

    assert_eq!(
        error,
        "feishu.doc.create accepts either payload.content or payload.content_path, not both"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_doc_create_tool_requires_docx_write_scope() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("doc-create-scope");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-create-scope",
        &["offline_access", "docx:document:readonly"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.create".to_owned(),
            payload: serde_json::json!({
                "title": "Release Plan"
            }),
        },
        &config,
    )
    .expect_err("feishu doc create should reject readonly grant");

    assert!(error.contains("feishu.doc.create requires Feishu scopes [docx:document]"));
    assert!(error.contains("update Feishu config if needed"));
    assert!(error.contains("loong feishu auth start --account <account>"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_uses_selected_grant_and_user_token() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-paragraph"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-paragraph",
                                            "block_type": 2,
                                            "text": {
                                                "elements": [{"text_run": {"content": "Follow-up note"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "blk_real_paragraph",
                                            "temporary_block_id": "tmp-paragraph"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "Follow-up note"
            }),
        },
        &config,
    )
    .expect("feishu doc append tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["document"]["document_id"], "doxcnExisting");
    assert_eq!(
        outcome.payload["document"]["url"],
        "https://open.feishu.cn/docx/doxcnExisting"
    );
    assert_eq!(outcome.payload["inserted_block_count"], 1);
    assert_eq!(outcome.payload["insert_batch_count"], 1);
    assert_eq!(outcome.payload["content_type"], "markdown");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.authorization.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("Bearer u-token-doc-append"),
            Some("Bearer u-token-doc-append"),
        ]
    );
    assert_eq!(
        requests[0].path,
        "/open-apis/docx/v1/documents/blocks/convert"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).expect("convert request json"),
        serde_json::json!({
            "content_type": "markdown",
            "content": "Follow-up note"
        })
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
    );
    assert_eq!(
        requests[1].query.as_deref(),
        Some("document_revision_id=-1")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).expect("descendant request json"),
        serde_json::json!({
            "children_id": ["tmp-paragraph"],
            "descendants": [
                {
                    "block_id": "tmp-paragraph",
                    "block_type": 2,
                    "text": {
                        "elements": [{"text_run": {"content": "Follow-up note"}}]
                    },
                    "children": []
                }
            ],
            "index": -1
        })
    );

    server.abort();
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_reads_html_content_path_under_safe_file_root() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-content-path");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("content-root");
    let content_path = file_root.join("docs/follow-up.html");
    fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    fs::write(&content_path, "<p>Follow-up note</p>").expect("write content fixture");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-paragraph"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-paragraph",
                                            "block_type": 2,
                                            "text": {
                                                "elements": [{"text_run": {"content": "Follow-up note"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "blk_real_paragraph",
                                            "temporary_block_id": "tmp-paragraph"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-path",
        &["offline_access", "docx:document"],
    );
    let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
    config.file_root = Some(file_root.clone());

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content_path": "docs/follow-up.html"
            }),
        },
        &config,
    )
    .expect("feishu doc append tool should read content_path");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content_type"], "html");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).expect("convert request json"),
        serde_json::json!({
            "content_type": "html",
            "content": "<p>Follow-up note</p>"
        })
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_batches_nested_insert_requests_over_limit() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-batch");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let first_level_block_ids = (0..1001)
        .map(|index| format!("tmp-block-{index:04}"))
        .collect::<Vec<_>>();
    let descendants = first_level_block_ids
        .iter()
        .map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        })
        .collect::<Vec<_>>();
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": first_level_block_ids,
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-batch",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "large converted payload"
            }),
        },
        &config,
    )
    .expect("feishu doc append batching should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1001);
    assert_eq!(outcome.payload["insert_batch_count"], 2);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    let first_descendant_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
    let second_descendant_body =
        serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
    assert_eq!(
        first_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );
    assert_eq!(
        first_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );
    assert_eq!(
        second_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );
    assert_eq!(
        second_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_recursively_inserts_single_oversized_subtree() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-recursive-subtree");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let child_ids = (0..1001)
        .map(|index| format!("tmp-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(child_ids.len() + 1);
    descendants.push(serde_json::json!({
        "block_id": "tmp-root",
        "block_type": 3,
        "heading1": {
            "elements": [{"text_run": {"content": "Large subtree"}}]
        },
        "children": child_ids,
    }));
    descendants.extend(child_ids.iter().map(|block_id| {
        serde_json::json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-root"],
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_root",
                                    "temporary_block_id": "tmp-root"
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-recursive",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "large single subtree"
            }),
        },
        &config,
    )
    .expect("feishu doc append recursive subtree insertion should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1002);
    assert_eq!(outcome.payload["insert_batch_count"], 3);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 4);
    let first_descendant_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
    let second_descendant_body =
        serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
    let third_descendant_body =
        serde_json::from_str::<Value>(&requests[3].body).expect("third descendant body json");

    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
    );
    assert_eq!(
        first_descendant_body["children_id"],
        serde_json::json!(["tmp-root"])
    );
    assert_eq!(
        first_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );
    assert_eq!(
        first_descendant_body["descendants"][0]["children"],
        serde_json::json!([])
    );

    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
    );
    assert_eq!(
        second_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );
    assert_eq!(
        second_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );

    assert_eq!(
        requests[3].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
    );
    assert_eq!(
        third_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );
    assert_eq!(
        third_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_recursively_inserts_deep_oversized_subtree() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-recursive-deep-subtree");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1001)
        .map(|index| format!("tmp-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 2);
    descendants.push(serde_json::json!({
        "block_id": "tmp-root",
        "block_type": 3,
        "heading1": {
            "elements": [{"text_run": {"content": "Deep oversized subtree"}}]
        },
        "children": ["tmp-mid"],
    }));
    descendants.push(serde_json::json!({
        "block_id": "tmp-mid",
        "block_type": 2,
        "text": {
            "elements": [{"text_run": {"content": "Large child subtree"}}]
        },
        "children": leaf_ids,
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        serde_json::json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-root"],
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_root",
                                    "temporary_block_id": "tmp-root"
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_mid",
                                    "temporary_block_id": "tmp-mid"
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-recursive-deep",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "deep large single subtree"
            }),
        },
        &config,
    )
    .expect("feishu doc append deep recursive subtree insertion should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1003);
    assert_eq!(outcome.payload["insert_batch_count"], 4);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 5);
    let first_descendant_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
    let second_descendant_body =
        serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
    let third_descendant_body =
        serde_json::from_str::<Value>(&requests[3].body).expect("third descendant body json");
    let fourth_descendant_body =
        serde_json::from_str::<Value>(&requests[4].body).expect("fourth descendant body json");

    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
    );
    assert_eq!(
        first_descendant_body["children_id"],
        serde_json::json!(["tmp-root"])
    );
    assert_eq!(
        first_descendant_body["descendants"],
        serde_json::json!([{
            "block_id": "tmp-root",
            "block_type": 3,
            "heading1": {
                "elements": [{"text_run": {"content": "Deep oversized subtree"}}]
            },
            "children": []
        }])
    );

    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
    );
    assert_eq!(
        second_descendant_body["children_id"],
        serde_json::json!(["tmp-mid"])
    );
    assert_eq!(
        second_descendant_body["descendants"],
        serde_json::json!([{
            "block_id": "tmp-mid",
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": "Large child subtree"}}]
            },
            "children": []
        }])
    );

    assert_eq!(
        requests[3].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant"
    );
    assert_eq!(
        third_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );
    assert_eq!(
        third_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );

    assert_eq!(
        requests[4].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant"
    );
    assert_eq!(
        fourth_descendant_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );
    assert_eq!(
        fourth_descendant_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_supports_oversized_table_subtree() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-table-subtree");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1000)
        .map(|index| format!("tmp-cell-1-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
    descendants.push(serde_json::json!({
        "block_id": "tmp-table",
        "block_type": 31,
        "table": {
            "property": {
                "row_size": 1,
                "column_size": 2
            }
        },
        "children": ["tmp-cell-1", "tmp-cell-2"],
    }));
    descendants.push(serde_json::json!({
        "block_id": "tmp-cell-1",
        "block_type": 32,
        "table_cell": {},
        "children": leaf_ids,
    }));
    descendants.push(serde_json::json!({
        "block_id": "tmp-cell-2",
        "block_type": 32,
        "table_cell": {},
        "children": [],
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        serde_json::json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-table"],
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_table",
                                    "block_type": 31,
                                    "children": ["blk_real_cell_1", "blk_real_cell_2"],
                                    "table": {
                                        "cells": ["blk_real_cell_1", "blk_real_cell_2"],
                                        "property": {
                                            "row_size": 1,
                                            "column_size": 2
                                        }
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-oversized-table",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "oversized table subtree"
            }),
        },
        &config,
    )
    .expect("feishu doc append oversized table subtree insertion should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1003);
    assert_eq!(outcome.payload["insert_batch_count"], 2);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);

    let create_table_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("create table body json");
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
    );
    assert_eq!(
        create_table_body["children"].as_array().map_or(0, Vec::len),
        1
    );
    assert_eq!(create_table_body["children"][0]["block_type"], 31);
    assert!(create_table_body["children"][0].get("block_id").is_none());
    assert!(create_table_body["children"][0].get("children").is_none());

    let first_cell_body =
        serde_json::from_str::<Value>(&requests[2].body).expect("first cell body json");
    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant"
    );
    assert_eq!(
        first_cell_body["children_id"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );
    assert_eq!(
        first_cell_body["descendants"]
            .as_array()
            .map_or(0, Vec::len),
        1000
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_supports_oversized_callout_subtree() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-callout-subtree");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1001)
        .map(|index| format!("tmp-callout-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 1);
    descendants.push(serde_json::json!({
        "block_id": "tmp-callout",
        "block_type": 19,
        "callout": {
            "emoji_id": "smile"
        },
        "children": leaf_ids,
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        serde_json::json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-callout"],
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_callout",
                                    "block_type": 19,
                                    "children": [],
                                    "callout": {
                                        "emoji_id": "smile"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-oversized-callout",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "oversized callout subtree"
            }),
        },
        &config,
    )
    .expect("feishu doc append oversized callout subtree insertion should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1002);
    assert_eq!(outcome.payload["insert_batch_count"], 3);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 4);
    let create_callout_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("create callout body json");
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
    );
    assert_eq!(
        create_callout_body["children"],
        serde_json::json!([{
            "block_type": 19,
            "callout": {
                "emoji_id": "smile"
            }
        }])
    );
    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
    );
    assert_eq!(
        requests[3].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_doc_append_tool_supports_oversized_grid_subtree() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-grid-subtree");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1000)
        .map(|index| format!("tmp-grid-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
    descendants.push(serde_json::json!({
        "block_id": "tmp-grid",
        "block_type": 24,
        "grid": {},
        "children": ["tmp-grid-col-1", "tmp-grid-col-2"],
    }));
    descendants.push(serde_json::json!({
        "block_id": "tmp-grid-col-1",
        "block_type": 25,
        "grid_column": {},
        "children": leaf_ids,
    }));
    descendants.push(serde_json::json!({
        "block_id": "tmp-grid-col-2",
        "block_type": 25,
        "grid_column": {},
        "children": [],
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        serde_json::json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = serde_json::json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-grid"],
            "blocks": descendants,
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request: Request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_grid",
                                    "block_type": 24,
                                    "children": ["blk_real_grid_col_1", "blk_real_grid_col_2"],
                                    "grid": {}
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-oversized-grid",
        &["offline_access", "docx:document"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "oversized grid subtree"
            }),
        },
        &config,
    )
    .expect("feishu doc append oversized grid subtree insertion should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["inserted_block_count"], 1003);
    assert_eq!(outcome.payload["insert_batch_count"], 2);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    let create_grid_body =
        serde_json::from_str::<Value>(&requests[1].body).expect("create grid body json");
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
    );
    assert_eq!(
        create_grid_body["children"],
        serde_json::json!([{
            "block_type": 24,
            "grid": {}
        }])
    );
    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_doc_append_tool_requires_docx_write_scope() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("doc-append-scope");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-doc-append-scope",
        &["offline_access", "docx:document:readonly"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.doc.append".to_owned(),
            payload: serde_json::json!({
                "url": "https://open.feishu.cn/docx/doxcnExisting",
                "content": "Follow-up note"
            }),
        },
        &config,
    )
    .expect_err("feishu doc append should reject readonly grant");

    assert!(error.contains("feishu.doc.append requires Feishu scopes [docx:document]"));
    assert!(error.contains("update Feishu config if needed"));
    assert!(error.contains("loong feishu auth start --account <account>"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_history_tool_uses_tenant_token_before_im_request() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-history");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-history"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "has_more": false,
                                "page_token": "",
                                "items": [{
                                    "message_id": "om_123",
                                    "chat_id": "oc_demo",
                                    "msg_type": "text",
                                    "create_time": "1700000000"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-history",
        &["offline_access", "im:message:readonly"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.history".to_owned(),
            payload: serde_json::json!({
                "container_id_type": "chat",
                "container_id": "oc_demo",
                "page_size": 20
            }),
        },
        &config,
    )
    .expect("feishu messages history tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");
    assert_eq!(outcome.payload["page"]["items"][0]["message_id"], "om_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[0].authorization, None);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-history")
    );
    let query = requests[1].query.as_deref().unwrap_or_default();
    assert!(query.contains("container_id_type=chat"));
    assert!(query.contains("container_id=oc_demo"));
    assert!(query.contains("page_size=20"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_history_tool_defaults_thread_container_and_account_from_internal_ingress()
{
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-history-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-history-ingress"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "has_more": false,
                                "page_token": "",
                                "items": [{
                                    "message_id": "om_thread_hist_1",
                                    "chat_id": "oc_ingress_history",
                                    "root_id": "omt_ingress_history",
                                    "msg_type": "text",
                                    "create_time": "1700000100"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-history-ingress",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.history".to_owned(),
            payload: serde_json::json!({
                "page_size": 20,
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_ingress_history",
                            "thread_id": "omt_ingress_history"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages history tool should infer thread container from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(
        outcome.payload["page"]["items"][0]["message_id"],
        "om_thread_hist_1"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    let query = requests[1].query.as_deref().unwrap_or_default();
    assert!(query.contains("container_id_type=thread"), "query={query}");
    assert!(
        query.contains("container_id=omt_ingress_history"),
        "query={query}"
    );
    assert!(query.contains("page_size=20"), "query={query}");

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_get_tool_uses_tenant_token_for_detail_request() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-get");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-detail"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_789",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_789",
                                    "chat_id": "oc_demo",
                                    "msg_type": "text",
                                    "sender": {
                                        "id": "ou_123",
                                        "sender_type": "user"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-detail",
        &["offline_access", "im:message.group_msg"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_789"
            }),
        },
        &config,
    )
    .expect("feishu messages get tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["message"]["message_id"], "om_789");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-detail")
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages/om_789");

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_get_tool_defaults_message_id_and_account_from_internal_ingress() {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-get-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-detail-ingress"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_ingress_detail",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_ingress_detail",
                                    "chat_id": "oc_demo",
                                    "msg_type": "text",
                                    "sender": {
                                        "id": "ou_shared",
                                        "sender_type": "user"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-detail-ingress",
        &["offline_access", "im:message.group_msg"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.get".to_owned(),
            payload: serde_json::json!({
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_detail"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages get tool should infer message id and account from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(
        outcome.payload["message"]["message_id"],
        "om_ingress_detail"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-detail-ingress")
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_ingress_detail"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_get_tool_accepts_legacy_group_message_read_scope() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-get-legacy-group-scope");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-detail-legacy"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_legacy",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_legacy",
                                    "chat_id": "oc_demo",
                                    "msg_type": "text",
                                    "sender": {
                                        "id": "ou_123",
                                        "sender_type": "user"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-detail-legacy",
        &["offline_access", "im:message.group_msg:readonly"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_legacy"
            }),
        },
        &config,
    )
    .expect("legacy group scope should remain accepted");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["message"]["message_id"], "om_legacy");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-detail-legacy")
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages/om_legacy");

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_downloads_message_resource_to_safe_file_root() {
    use std::fs;

    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::Response,
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        axum::Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_resource_123/resources/file_demo_456",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            axum::http::header::CONTENT_TYPE,
                            HeaderValue::from_static("application/pdf"),
                        );
                        headers.insert(
                            axum::http::header::CONTENT_DISPOSITION,
                            HeaderValue::from_static("attachment; filename=\"spec-sheet.pdf\""),
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("pdf-demo-bytes"))
                            .map(|mut response| {
                                *response.headers_mut() = headers;
                                response
                            })
                            .expect("build binary response")
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-resource",
        &["offline_access", "im:message:readonly"],
    );
    let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
    config.file_root = Some(file_root.clone());

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_resource_123",
                "file_key": "file_demo_456",
                "type": "file",
                "save_as": "artifacts/specs/spec-sheet.pdf"
            }),
        },
        &config,
    )
    .expect("feishu message resource tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["message_id"], "om_resource_123");
    assert_eq!(outcome.payload["file_key"], "file_demo_456");
    assert_eq!(outcome.payload["resource_type"], "file");
    assert_eq!(outcome.payload["content_type"], "application/pdf");
    assert_eq!(outcome.payload["file_name"], "spec-sheet.pdf");
    assert_eq!(outcome.payload["bytes_written"], 14);

    let expected_path = file_root.join("artifacts/specs/spec-sheet.pdf");
    let canonical_expected_path =
        dunce::canonicalize(&expected_path).expect("canonicalize downloaded file");
    assert_eq!(
        outcome.payload["path"].as_str(),
        Some(canonical_expected_path.display().to_string().as_str())
    );
    assert_eq!(
        fs::read(&expected_path).expect("read downloaded file"),
        b"pdf-demo-bytes"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-resource")
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_resource_123/resources/file_demo_456"
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=file"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_defaults_message_id_account_and_single_resource_from_internal_ingress()
 {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::Response,
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        axum::Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource-ingress"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_ingress_resource/resources/img_ingress_456",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            axum::http::header::CONTENT_TYPE,
                            HeaderValue::from_static("image/png"),
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("png-demo-bytes"))
                            .map(|mut response| {
                                *response.headers_mut() = headers;
                                response
                            })
                            .expect("build binary response")
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-ingress",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root.clone()),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "save_as": "artifacts/images/incoming.png",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_resource",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_ingress_456"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu message resource tool should infer message id and account from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["message_id"], "om_ingress_resource");
    assert_eq!(outcome.payload["resource_type"], "image");

    let saved_path = outcome.payload["path"]
        .as_str()
        .expect("resource output path");
    assert!(
        std::path::Path::new(saved_path).ends_with(
            std::path::Path::new("artifacts")
                .join("images")
                .join("incoming.png")
        ),
        "saved_path {saved_path} should end with artifacts/images/incoming.png",
    );
    assert_eq!(
        fs::read(saved_path).expect("read downloaded image"),
        b"png-demo-bytes"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-resource-ingress")
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_ingress_resource/resources/img_ingress_456"
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=image"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_accepts_audio_alias_for_single_file_ingress_resource() {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::Response,
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-audio-alias");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        axum::Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource-audio-alias"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_ingress_audio/resources/file_audio_456",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            axum::http::header::CONTENT_TYPE,
                            HeaderValue::from_static("audio/ogg"),
                        );
                        headers.insert(
                            axum::http::header::CONTENT_DISPOSITION,
                            HeaderValue::from_static("attachment; filename=\"voice.ogg\""),
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("voice-demo-bytes"))
                            .map(|mut response| {
                                *response.headers_mut() = headers;
                                response
                            })
                            .expect("build binary response")
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-audio-alias",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root.clone()),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some(base_url),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                default_account: Some("work".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "type": "audio",
                "save_as": "artifacts/audio/voice.ogg",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_audio",
                            "resources": [
                                {
                                    "type": "file",
                                    "file_key": "file_audio_456"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu message resource tool should accept audio alias");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["message_id"], "om_ingress_audio");
    assert_eq!(outcome.payload["file_key"], "file_audio_456");
    assert_eq!(outcome.payload["resource_type"], "file");
    assert_eq!(outcome.payload["content_type"], "audio/ogg");
    assert_eq!(outcome.payload["file_name"], "voice.ogg");

    let saved_path = outcome.payload["path"]
        .as_str()
        .expect("resource output path");
    assert!(
        std::path::Path::new(saved_path).ends_with(
            std::path::Path::new("artifacts")
                .join("audio")
                .join("voice.ogg")
        ),
        "saved_path {saved_path} should end with artifacts/audio/voice.ogg",
    );
    assert_eq!(
        fs::read(saved_path).expect("read downloaded audio"),
        b"voice-demo-bytes"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-resource-audio-alias")
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_ingress_audio/resources/file_audio_456"
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=file"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_infers_file_key_from_unique_type_in_multi_resource_ingress()
 {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::Response,
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-type-infer");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        axum::Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource-multi-type"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_ingress_media/resources/img_media_456",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            axum::http::header::CONTENT_TYPE,
                            HeaderValue::from_static("image/png"),
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("png-media-preview"))
                            .map(|mut response| {
                                *response.headers_mut() = headers;
                                response
                            })
                            .expect("build binary response")
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-multi-type",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root.clone()),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some(base_url),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "type": "image",
                "save_as": "artifacts/media/preview.png",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_media",
                            "resources": [
                                {
                                    "type": "file",
                                    "file_key": "file_media_123",
                                    "file_name": "clip.mp4"
                                },
                                {
                                    "type": "image",
                                    "file_key": "img_media_456"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("unique ingress type should infer file key");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["message_id"], "om_ingress_media");
    assert_eq!(outcome.payload["file_key"], "img_media_456");
    assert_eq!(outcome.payload["resource_type"], "image");

    let saved_path = outcome.payload["path"]
        .as_str()
        .expect("resource output path");
    assert!(
        std::path::Path::new(saved_path).ends_with(
            std::path::Path::new("artifacts")
                .join("media")
                .join("preview.png")
        ),
        "saved_path {saved_path} should end with artifacts/media/preview.png",
    );
    assert_eq!(
        fs::read(saved_path).expect("read downloaded image"),
        b"png-media-preview"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_ingress_media/resources/img_media_456"
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=image"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_infers_type_from_matching_file_key_in_multi_resource_ingress()
 {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::Response,
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-key-infer");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        axum::Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource-multi-key"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_ingress_post/resources/img_post_456",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            axum::http::header::CONTENT_TYPE,
                            HeaderValue::from_static("image/jpeg"),
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("jpeg-post-image"))
                            .map(|mut response| {
                                *response.headers_mut() = headers;
                                response
                            })
                            .expect("build binary response")
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-multi-key",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root.clone()),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some(base_url),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "file_key": "img_post_456",
                "save_as": "artifacts/post/image.jpg",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_post",
                            "resources": [
                                {
                                    "type": "file",
                                    "file_key": "file_post_123",
                                    "file_name": "report.pdf"
                                },
                                {
                                    "type": "image",
                                    "file_key": "img_post_456"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("matching ingress file key should infer type");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["message_id"], "om_ingress_post");
    assert_eq!(outcome.payload["file_key"], "img_post_456");
    assert_eq!(outcome.payload["resource_type"], "image");

    let saved_path = outcome.payload["path"]
        .as_str()
        .expect("resource output path");
    assert!(
        std::path::Path::new(saved_path).ends_with(
            std::path::Path::new("artifacts")
                .join("post")
                .join("image.jpg")
        ),
        "saved_path {saved_path} should end with artifacts/post/image.jpg",
    );
    assert_eq!(
        fs::read(saved_path).expect("read downloaded image"),
        b"jpeg-post-image"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_ingress_post/resources/img_post_456"
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=image"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_rejects_type_only_when_multi_resource_ingress_has_multiple_matches()
 {
    use std::collections::BTreeMap;
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-type-ambiguous");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-multi-type-ambiguous",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "type": "image",
                "save_as": "artifacts/post/ambiguous.jpg",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_post",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_post_111"
                                },
                                {
                                    "type": "image",
                                    "file_key": "img_post_222"
                                },
                                {
                                    "type": "file",
                                    "file_key": "file_post_333",
                                    "file_name": "report.pdf"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("ambiguous type-only selection should be rejected");

    assert!(
        error.contains("payload.type matches multiple current Feishu ingress resources")
            && error.contains("img_post_111")
            && error.contains("img_post_222")
            && error.contains("payload.file_key")
            && error.contains("resource_inventory"),
        "unexpected error: {error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_does_not_infer_ingress_resource_when_message_id_overrides_current_ingress_message()
 {
    use std::collections::BTreeMap;
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-override-message-no-infer");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-override-no-infer",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_other_message",
                "type": "image",
                "save_as": "artifacts/post/override.jpg",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_post",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_post_111"
                                },
                                {
                                    "type": "file",
                                    "file_key": "file_post_333",
                                    "file_name": "report.pdf"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("explicit message override should disable ingress resource inference");

    assert!(
            error.contains("requires payload.file_key")
                && error.contains("payload.message_id `om_other_message` differs")
                && error.contains("current Feishu ingress message `om_ingress_post`")
                && error.contains(
                    "defaults only apply when payload.message_id is omitted or matches the current message"
                ),
            "unexpected error: {error}"
        );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_rejects_explicit_resource_pair_that_conflicts_with_current_ingress_resource()
 {
    use std::collections::BTreeMap;
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-explicit-pair-conflict");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-pair-conflict",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "file_key": "img_post_111",
                "type": "file",
                "save_as": "artifacts/post/conflict.bin",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_post",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_post_111"
                                },
                                {
                                    "type": "file",
                                    "file_key": "file_post_333",
                                    "file_name": "report.pdf"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("mismatched explicit file_key and type should be rejected");

    assert!(
        error.contains("payload.type conflicts")
            && error.contains("type=image")
            && error.contains("file_key=img_post_111"),
        "unexpected error: {error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_rejects_ambiguous_ingress_resource_defaults() {
    use std::collections::BTreeMap;
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-ingress-ambiguous");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-ingress-ambiguous",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "save_as": "artifacts/images/ambiguous.png",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_resource",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_ingress_456"
                                },
                                {
                                    "type": "file",
                                    "file_key": "file_ingress_789",
                                    "file_name": "report.pdf"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("ambiguous ingress resources should require explicit selection");

    assert!(
        error.contains("multiple Feishu message resources"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("img_ingress_456")
            && error.contains("file_ingress_789")
            && error.contains("report.pdf")
            && error.contains("resource_inventory"),
        "unexpected error: {error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_reports_current_ingress_resource_on_file_key_conflict() {
    use std::collections::BTreeMap;
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-file-key-conflict");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-resource-conflict",
        &["offline_access", "im:message:readonly"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(file_root),
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_shared".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-work".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_ingress_resource",
                "file_key": "img_other_999",
                "save_as": "artifacts/images/conflict.png",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_ingress_resource",
                            "resources": [
                                {
                                    "type": "image",
                                    "file_key": "img_ingress_456"
                                }
                            ]
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("conflicting file key should be rejected");

    assert!(
        error.contains("payload.file_key conflicts")
            && error.contains("type=image")
            && error.contains("file_key=img_ingress_456"),
        "unexpected error: {error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_get_tool_rejects_paths_that_escape_file_root() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-escape");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("downloads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-resource-escape",
        &["offline_access", "im:message:readonly"],
    );
    let mut config =
        build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
    config.file_root = Some(file_root);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.resource.get".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_resource_escape",
                "file_key": "file_escape_456",
                "type": "file",
                "save_as": "../escape.pdf"
            }),
        },
        &config,
    )
    .expect_err("path escape should be rejected");

    assert!(
        error.contains("escapes configured file root"),
        "unexpected error: {error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_whoami_tool_refreshes_expired_grant_before_fetching_user_profile() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::{get, post},
    };

    let temp_dir = unique_feishu_tool_temp_dir("whoami-refresh");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/authen/v2/oauth/token",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "access_token": "u-token-refreshed",
                            "refresh_token": "r-token-next",
                            "expires_in": 7200,
                            "refresh_token_expires_in": 2592000,
                            "scope": "offline_access search:message calendar:calendar:readonly"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/authen/v1/user_info",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "name": "Alice Refreshed",
                                "open_id": "ou_123",
                                "union_id": "on_456",
                                "user_id": "u_789",
                                "tenant_key": "tenant_x"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let now_s = crate::channel::feishu::api::unix_ts_now();
    let store = crate::channel::feishu::api::FeishuTokenStore::new(sqlite_path.clone());
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice Old".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-expired".to_owned(),
            refresh_token: "r-token-old".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
            ]),
            access_expires_at_s: now_s - 10,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s - 3600,
        })
        .expect("save expired grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select grant");
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: serde_json::json!({}),
        },
        &config,
    )
    .expect("feishu whoami tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");
    assert_eq!(outcome.payload["principal"]["name"], "Alice Refreshed");
    assert_eq!(outcome.payload["user_info"]["name"], "Alice Refreshed");

    let stored = store
        .load_grant("feishu_main", "ou_123")
        .expect("load refreshed grant")
        .expect("refreshed grant should still exist");
    assert_eq!(stored.access_token, "u-token-refreshed");
    assert_eq!(stored.refresh_token, "r-token-next");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/open-apis/authen/v2/oauth/token");
    assert!(
        requests[0]
            .body
            .contains("\"grant_type\":\"refresh_token\"")
    );
    assert_eq!(requests[1].path, "/open-apis/authen/v1/user_info");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer u-token-refreshed")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_whoami_tool_includes_configured_account_in_outcome_for_account_alias() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::get,
    };

    let temp_dir = unique_feishu_tool_temp_dir("whoami-configured-account-outcome");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/authen/v1/user_info",
        get({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "name": "Alice Alias",
                            "open_id": "ou_alias",
                            "union_id": "on_alias",
                            "user_id": "u_alias",
                            "tenant_key": "tenant_alias"
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_secondary",
        "ou_alias",
        "u-token-alias",
        &["offline_access"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_secondary".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline(
                            "cli_secondary".to_owned(),
                        )),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-secondary".to_owned(),
                        )),
                        base_url: Some(base_url),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: json!({
                "account_id": "work",
                "open_id": "ou_alias"
            }),
        },
        &config,
    )
    .expect("feishu whoami tool should succeed for configured account alias");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["account_id"], "feishu_secondary");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_alias");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/authen/v1/user_info");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-alias")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_whoami_tool_suggests_account_scoped_auth_start_when_no_grant_exists() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("whoami-no-grant");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: json!({}),
        },
        &config,
    )
    .expect_err("missing Feishu grant should fail");

    assert!(error.contains("loong feishu auth start --account feishu_main"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_whoami_tool_uses_configured_account_id_in_auth_hint() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("whoami-configured-account-hint");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_secondary".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline(
                            "cli_secondary".to_owned(),
                        )),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-secondary".to_owned(),
                        )),
                        base_url: Some("http://127.0.0.1:9".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: json!({
                "account_id": "work"
            }),
        },
        &config,
    )
    .expect_err("missing Feishu grant should fail");

    assert!(error.contains("loong feishu auth start --account work"));
    assert!(!error.contains("--account feishu_secondary"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_whoami_tool_suggests_auth_select_when_multiple_grants_are_available() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("whoami-multi-grant");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let store = crate::channel::feishu::api::FeishuTokenStore::new(sqlite_path.clone());
    let now_s = crate::channel::feishu::api::unix_ts_now();
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_123".to_owned()),
                user_id: Some("u_123".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-123".to_owned(),
            refresh_token: "r-token-123".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        })
        .expect("save first grant");
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_456".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_456".to_owned()),
                name: Some("Bob".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("bob@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-456".to_owned(),
            refresh_token: "r-token-456".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s + 1,
        })
        .expect("save second grant");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: json!({}),
        },
        &config,
    )
    .expect_err("ambiguous Feishu grant selection should fail");

    assert!(error.contains("loong feishu auth select --account feishu_main --open-id <open_id>"));
    assert!(error.contains("ou_123"));
    assert!(error.contains("ou_456"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_whoami_tool_reports_available_open_ids_for_missing_explicit_open_id() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("whoami-missing-explicit-open-id");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let store = crate::channel::feishu::api::FeishuTokenStore::new(sqlite_path.clone());
    let now_s = crate::channel::feishu::api::unix_ts_now();
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_123".to_owned()),
                user_id: Some("u_123".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-123".to_owned(),
            refresh_token: "r-token-123".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s,
        })
        .expect("save first grant");
    store
        .save_grant(&crate::channel::feishu::api::FeishuGrant {
            principal: crate::channel::feishu::api::FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_456".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_456".to_owned()),
                name: Some("Bob".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("bob@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token-456".to_owned(),
            refresh_token: "r-token-456".to_owned(),
            scopes: crate::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
                "offline_access",
            ]),
            access_expires_at_s: now_s + 3600,
            refresh_expires_at_s: now_s + 86_400,
            refreshed_at_s: now_s + 1,
        })
        .expect("save second grant");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "feishu.whoami".to_owned(),
            payload: json!({
                "open_id": "ou_missing"
            }),
        },
        &config,
    )
    .expect_err("missing explicit open_id should fail");

    assert!(error.contains("open_id `ou_missing`"));
    assert!(error.contains("available open_ids: ou_456, ou_123"));
    assert!(error.contains("loong feishu auth select --account feishu_main --open-id <open_id>"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_search_tool_uses_user_token_directly() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-search");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/search/v2/message",
        post({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "items": ["om_1", "om_2"],
                            "page_token": "next-search",
                            "has_more": true
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-search",
        &["offline_access", "search:message"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.search".to_owned(),
            payload: serde_json::json!({
                "query": "incident",
                "user_id_type": "open_id",
                "page_size": 10,
                "chat_ids": ["oc_demo"],
                "from_ids": ["ou_123"],
                "message_type": "text"
            }),
        },
        &config,
    )
    .expect("feishu messages search tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["page"]["items"][0], "om_1");
    assert_eq!(outcome.payload["page"]["has_more"], true);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/search/v2/message");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-search")
    );
    assert!(requests[0].query.as_deref().is_some_and(|query| {
        query.contains("user_id_type=open_id") && query.contains("page_size=10")
    }));
    assert!(requests[0].body.contains("\"query\":\"incident\""));
    assert!(requests[0].body.contains("\"chat_ids\":[\"oc_demo\"]"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_bitable_list_tool_returns_top_level_tables_page() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::get,
    };

    let temp_dir = unique_feishu_tool_temp_dir("bitable-list-tables");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/bitable/v1/apps/app_demo/tables",
        get({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "items": [{
                                "table_id": "tbl_1",
                                "name": "Tasks",
                                "revision": 3
                            }],
                            "page_token": "page_next",
                            "has_more": true
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-bitable-list",
        &["offline_access", "base:table:read"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.bitable.list".to_owned(),
            payload: serde_json::json!({
                "app_token": "app_demo",
                "page_size": 20,
                "page_token": "page_current"
            }),
        },
        &config,
    )
    .expect("feishu bitable list tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tables"][0]["table_id"], "tbl_1");
    assert_eq!(outcome.payload["has_more"], true);
    assert_eq!(outcome.payload["page_token"], "page_next");
    assert!(outcome.payload.get("result").is_none());

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].path,
        "/open-apis/bitable/v1/apps/app_demo/tables"
    );
    assert!(requests[0].query.as_deref().is_some_and(|query| {
        query.contains("page_size=20") && query.contains("page_token=page_current")
    }));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_search_tool_defaults_chat_scope_and_account_from_internal_ingress() {
    use std::collections::BTreeMap;
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-search-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/search/v2/message",
        post({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "items": ["om_ingress_search_1"],
                            "page_token": "",
                            "has_more": false
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-search-ingress",
        &["offline_access", "search:message"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.search".to_owned(),
            payload: serde_json::json!({
                "query": "incident",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_ingress_search"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages search tool should infer chat scope from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["page"]["items"][0], "om_ingress_search_1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/search/v2/message");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-search-ingress")
    );
    assert!(
        requests[0]
            .body
            .contains("\"chat_ids\":[\"oc_ingress_search\"]")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_uses_tenant_token_and_default_receive_id_type() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_1",
                                "root_id": "om_sent_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "text": "ship it"
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_1");
    assert_eq!(outcome.payload["delivery"]["mode"], "send");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "text");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-send")
    );
    assert!(
        requests[1]
            .query
            .as_deref()
            .is_some_and(|query| { query.contains("receive_id_type=chat_id") })
    );
    assert!(requests[1].body.contains("\"receive_id\":\"oc_demo\""));
    assert!(requests[1].body.contains("\"msg_type\":\"text\""));
    assert!(requests[1].body.contains("\\\"text\\\":\\\"ship it\\\""));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_uses_tenant_token_and_card_mode() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-card");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-card"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_card_1",
                                "root_id": "om_sent_card_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-card",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "text": "ship it",
                "as_card": true
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should succeed in card mode");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_card_1");
    assert_eq!(outcome.payload["delivery"]["mode"], "send");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "interactive");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-send-card")
    );
    let request_body = requests[1].body.as_str();
    assert!(request_body.contains("\"msg_type\":\"interactive\""));
    assert!(request_body.contains("\\\"schema\\\":\\\"2.0\\\""));
    assert!(request_body.contains("\\\"tag\\\":\\\"markdown\\\""));
    assert!(request_body.contains("\\\"content\\\":\\\"ship it\\\""));
    assert!(
        !request_body.contains("\\\"card\\\":"),
        "interactive send should serialize the card directly without a card wrapper"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_defaults_receive_id_and_account_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-ingress"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_ingress_1",
                                "root_id": "om_sent_ingress_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_secondary",
        "ou_secondary",
        "u-token-secondary",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                account_id: Some("feishu_primary".to_owned()),
                app_id: Some(loong_contracts::SecretRef::Inline("cli_primary".to_owned())),
                app_secret: Some(loong_contracts::SecretRef::Inline(
                    "app-secret-primary".to_owned(),
                )),
                receive_id_type: "open_id".to_owned(),
                accounts: BTreeMap::from([(
                    "work".to_owned(),
                    crate::config::FeishuAccountConfig {
                        account_id: Some("feishu_secondary".to_owned()),
                        app_id: Some(loong_contracts::SecretRef::Inline(
                            "cli_secondary".to_owned(),
                        )),
                        app_secret: Some(loong_contracts::SecretRef::Inline(
                            "app-secret-secondary".to_owned(),
                        )),
                        base_url: Some(base_url),
                        receive_id_type: Some("chat_id".to_owned()),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "text": "ship by ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_secondary",
                            "conversation_id": "oc_ingress_send"
                        },
                        "delivery": {
                            "source_message_id": "om_source_send"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should default from internal ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["account_id"], "feishu_secondary");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["principal"]["open_id"], "ou_secondary");
    assert_eq!(outcome.payload["delivery"]["receive_id"], "oc_ingress_send");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(
        requests[1]
            .query
            .as_deref()
            .is_some_and(|query| { query.contains("receive_id_type=chat_id") })
    );
    assert!(
        requests[1]
            .body
            .contains("\"receive_id\":\"oc_ingress_send\"")
    );
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"ship by ingress\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_prefers_configured_account_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-configured-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-configured"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_send_configured_1",
                                "root_id": "om_send_configured_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-send-configured",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            receive_id_type: Some("chat_id".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            receive_id_type: Some("open_id".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "text": "send from configured ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_configured_send"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should use configured account from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(outcome.payload["account_id"], "feishu_shared");
    assert_eq!(
        outcome.payload["delivery"]["receive_id"],
        "oc_configured_send"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-send-configured")
    );
    assert!(
        requests[1]
            .query
            .as_deref()
            .is_some_and(|query| query.contains("receive_id_type=chat_id"))
    );
    assert!(
        requests[1]
            .body
            .contains("\"receive_id\":\"oc_configured_send\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_passes_uuid_to_api() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-uuid");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-uuid"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_uuid_1",
                                "root_id": "om_sent_uuid_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-uuid",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "text": "ship with uuid",
                "uuid": "send-uuid-1"
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should pass uuid");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_uuid_1");
    assert_eq!(outcome.payload["delivery"]["uuid"], "send-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(requests[1].body.contains("\"uuid\":\"send-uuid-1\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"ship with uuid\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_supports_post_content() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-post");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-post"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_post_1",
                                "root_id": "om_sent_post_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-post",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "post": {
                    "zh_cn": {
                        "title": "Ship update",
                        "content": [[
                            {
                                "tag": "text",
                                "text": "rich ship"
                            },
                            {
                                "tag": "a",
                                "text": "Open Platform",
                                "href": "https://open.feishu.cn"
                            }
                        ]]
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should support post content");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_post_1");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "post");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(requests[1].body.contains("\"msg_type\":\"post\""));
    assert!(requests[1].body.contains("\\\"zh_cn\\\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"title\\\":\\\"Ship update\\\"")
    );
    assert!(requests[1].body.contains("\\\"text\\\":\\\"rich ship\\\""));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_supports_image_key() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-image");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-image"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_image_1",
                                "root_id": "om_sent_image_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-image",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "image_key": "img_v2_demo"
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should support image_key");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_image_1");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "image");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(requests[1].body.contains("\"msg_type\":\"image\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"image_key\\\":\\\"img_v2_demo\\\"")
    );

    server.abort();
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_send_tool_uploads_image_path_under_safe_file_root_and_sends_image_message()
{
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-image-path");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("uploads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let image_path = file_root.join("assets/demo.png");
    fs::create_dir_all(image_path.parent().expect("image path parent"))
        .expect("create image parent");
    fs::write(&image_path, b"png-demo-bytes").expect("write image fixture");

    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-image-path"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/images",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "image_key": "img_uploaded_from_path"
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_sent_image_path_1",
                                "root_id": "om_sent_image_path_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-image-path",
        &["offline_access", "im:message:send_as_bot"],
    );
    let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
    config.file_root = Some(file_root);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "image_path": "assets/demo.png"
            }),
        },
        &config,
    )
    .expect("feishu messages send tool should upload image path");

    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["delivery"]["message_id"],
        "om_sent_image_path_1"
    );
    assert_eq!(outcome.payload["delivery"]["msg_type"], "image");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/images");
    assert_eq!(requests[2].path, "/open-apis/im/v1/messages");
    assert!(
        requests[1].body.contains("name=\"image_type\"") && requests[1].body.contains("message")
    );
    assert!(requests[1].body.contains("filename=\"demo.png\""));
    assert!(requests[2].body.contains("\"msg_type\":\"image\""));
    assert!(
        requests[2]
            .body
            .contains("\\\"image_key\\\":\\\"img_uploaded_from_path\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_messages_send_tool_rejects_mixed_text_and_post_content() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-post-mixed");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store =
        seed_feishu_tool_grant(&sqlite_path, "u-token-send-post-mixed", &["offline_access"]);
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "text": "plain text",
                "post": {
                    "zh_cn": {
                        "title": "Ship update",
                        "content": [[{
                            "tag": "text",
                            "text": "rich ship"
                        }]]
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("mixed text and post content should fail");

    assert!(error.contains("payload.text"));
    assert!(error.contains("payload.post"));
    assert!(error.contains("not both"));
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[test]
fn feishu_messages_send_tool_rejects_mixed_image_key_and_image_path() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-image-mixed-source");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-image-mixed-source",
        &["offline_access"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "image_key": "img_v2_demo",
                "image_path": "assets/demo.png"
            }),
        },
        &config,
    )
    .expect_err("mixed image key and image path should fail");

    assert_eq!(
        error,
        "feishu.messages.send accepts either payload.image_key or payload.image_path, not both"
    );
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[test]
fn feishu_messages_send_tool_rejects_file_type_without_file_path() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-file-type-without-path");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-file-type-without-path",
        &["offline_access"],
    );
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "file_key": "file_v2_demo",
                "file_type": "stream"
            }),
        },
        &config,
    )
    .expect_err("file type without file path should fail");

    assert_eq!(
        error,
        "feishu.messages.send only allows payload.file_type with payload.file_path"
    );
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[test]
fn feishu_messages_send_tool_rejects_file_path_that_escapes_safe_file_root() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-file-path-escape");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("uploads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-send-file-path-escape",
        &["offline_access"],
    );
    let mut config =
        build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
    config.file_root = Some(file_root.clone());

    let escape_target = file_root
        .parent()
        .expect("temp dir parent")
        .join("outside.txt");
    fs::write(&escape_target, b"not allowed").expect("write outside file");

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "file_path": "../outside.txt"
            }),
        },
        &config,
    )
    .expect_err("escaped file path should fail");

    assert!(error.contains("escapes configured file root"));
    assert!(error.contains("outside.txt"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_messages_send_tool_requires_confirmed_write_scope() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-scope");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(&sqlite_path, "u-token-send-scope", &["offline_access"]);
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "receive_id": "oc_demo",
                "text": "ship it"
            }),
        },
        &config,
    )
    .expect_err("write scope enforcement should reject grants without message send scopes");

    assert!(
            error.contains("feishu.messages.send requires at least one Feishu scope [im:message, im:message:send_as_bot, im:message:send]"),
            "error={error}"
        );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_uses_tenant_token_and_card_mode() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_1/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_1",
                                "root_id": "om_parent_1",
                                "parent_id": "om_parent_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_1",
                "text": "on it",
                "as_card": true
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_1");
    assert_eq!(outcome.payload["delivery"]["mode"], "reply");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "interactive");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_1/reply"
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-reply")
    );
    let request_body = requests[1].body.as_str();
    assert!(request_body.contains("\"msg_type\":\"interactive\""));
    assert!(request_body.contains("\\\"schema\\\":\\\"2.0\\\""));
    assert!(request_body.contains("\\\"tag\\\":\\\"markdown\\\""));
    assert!(request_body.contains("\\\"content\\\":\\\"on it\\\""));
    assert!(
        !request_body.contains("\\\"card\\\":"),
        "interactive reply should serialize the card directly without a card wrapper"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_passes_reply_in_thread_to_api() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-thread"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_thread/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_thread_1",
                                "root_id": "om_parent_thread",
                                "parent_id": "om_parent_thread"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-thread",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_thread",
                "text": "threaded reply",
                "reply_in_thread": true
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should support reply_in_thread");

    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["delivery"]["message_id"],
        "om_reply_thread_1"
    );
    assert_eq!(
        outcome.payload["delivery"]["reply_to_message_id"],
        "om_parent_thread"
    );
    assert_eq!(outcome.payload["delivery"]["reply_in_thread"], true);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_thread/reply"
    );
    assert!(requests[1].body.contains("\"reply_in_thread\":true"));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"threaded reply\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_passes_uuid_to_api() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-uuid");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-uuid"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_uuid/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_uuid_1",
                                "root_id": "om_parent_uuid",
                                "parent_id": "om_parent_uuid"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-uuid",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_uuid",
                "text": "reply with uuid",
                "uuid": "reply-uuid-1"
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should pass uuid");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_uuid_1");
    assert_eq!(outcome.payload["delivery"]["uuid"], "reply-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_uuid/reply"
    );
    assert!(requests[1].body.contains("\"uuid\":\"reply-uuid-1\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"reply with uuid\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_supports_post_content() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-post");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-post"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_post/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_post_1",
                                "root_id": "om_parent_post",
                                "parent_id": "om_parent_post"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-post",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_post",
                "post": {
                    "zh_cn": {
                        "title": "Thread update",
                        "content": [[{
                            "tag": "text",
                            "text": "rich reply"
                        }]]
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should support post content");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_post_1");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "post");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_post/reply"
    );
    assert!(requests[1].body.contains("\"msg_type\":\"post\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"title\\\":\\\"Thread update\\\"")
    );
    assert!(requests[1].body.contains("\\\"text\\\":\\\"rich reply\\\""));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_defaults_reply_in_thread_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread-ingress-default");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-thread-ingress-default"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_source_thread_ingress/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_thread_ingress_1",
                                "root_id": "om_thread_ingress",
                                "parent_id": "om_source_thread_ingress"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-thread-ingress-default",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "text": "reply from threaded ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "conversation_id": "oc_demo",
                            "thread_id": "om_thread_ingress"
                        },
                        "delivery": {
                            "source_message_id": "om_source_thread_ingress",
                            "thread_root_id": "om_thread_ingress"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should default reply_in_thread from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["reply_in_thread"], true);
    assert_eq!(
        outcome.payload["delivery"]["reply_to_message_id"],
        "om_source_thread_ingress"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_source_thread_ingress/reply"
    );
    assert!(requests[1].body.contains("\"reply_in_thread\":true"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_explicit_false_overrides_ingress_thread_hint() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread-ingress-explicit-false");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-thread-ingress-false"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_source_thread_ingress_false/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_thread_ingress_false_1",
                                "root_id": "om_source_thread_ingress_false",
                                "parent_id": "om_source_thread_ingress_false"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-thread-ingress-false",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "text": "reply from threaded ingress but not in thread",
                "reply_in_thread": false,
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "conversation_id": "oc_demo",
                            "thread_id": "om_thread_ingress_false"
                        },
                        "delivery": {
                            "source_message_id": "om_source_thread_ingress_false",
                            "thread_root_id": "om_thread_ingress_false"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should honor explicit false");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["reply_in_thread"], false);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_source_thread_ingress_false/reply"
    );
    assert!(!requests[1].body.contains("\"reply_in_thread\":true"));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_supports_file_key() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-file");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-file"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_file/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_file_1",
                                "root_id": "om_parent_file",
                                "parent_id": "om_parent_file"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-file",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_file",
                "file_key": "file_v2_demo"
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should support file_key");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_file_1");
    assert_eq!(outcome.payload["delivery"]["msg_type"], "file");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_file/reply"
    );
    assert!(requests[1].body.contains("\"msg_type\":\"file\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"file_key\\\":\\\"file_v2_demo\\\"")
    );

    server.abort();
}

#[cfg(all(
    feature = "feishu-integration",
    feature = "channel-feishu",
    feature = "tool-file"
))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_uploads_file_path_under_safe_file_root_and_replies_with_file_message()
 {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-file-path");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let file_root = temp_dir.join("uploads-root");
    fs::create_dir_all(&file_root).expect("create file root");
    let file_path = file_root.join("docs/spec-sheet.pdf");
    fs::create_dir_all(file_path.parent().expect("file path parent")).expect("create file parent");
    fs::write(&file_path, b"pdf-demo-bytes").expect("write file fixture");

    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-file-path"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/files",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "file_key": "file_uploaded_from_path"
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_file_path/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_file_path_1",
                                "root_id": "om_parent_file_path",
                                "parent_id": "om_parent_file_path"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-file-path",
        &["offline_access", "im:message:send_as_bot"],
    );
    let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
    config.file_root = Some(file_root);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "message_id": "om_parent_file_path",
                "file_path": "docs/spec-sheet.pdf",
                "file_type": "stream"
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should upload file path");

    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["delivery"]["message_id"],
        "om_reply_file_path_1"
    );
    assert_eq!(outcome.payload["delivery"]["msg_type"], "file");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/files");
    assert_eq!(
        requests[2].path,
        "/open-apis/im/v1/messages/om_parent_file_path/reply"
    );
    assert!(requests[1].body.contains("name=\"file_type\""));
    assert!(requests[1].body.contains("stream"));
    assert!(requests[1].body.contains("filename=\"spec-sheet.pdf\""));
    assert!(requests[2].body.contains("\"msg_type\":\"file\""));
    assert!(
        requests[2]
            .body
            .contains("\\\"file_key\\\":\\\"file_uploaded_from_path\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_defaults_message_id_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-ingress"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_source_ingress/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_ingress_1",
                                "root_id": "om_source_ingress",
                                "parent_id": "om_source_ingress"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-ingress",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "text": "reply from ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "source_message_id": "om_source_ingress",
                            "parent_message_id": "om_parent_fallback"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should default message id from internal ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["delivery"]["mode"], "reply");
    assert_eq!(
        outcome.payload["delivery"]["reply_to_message_id"],
        "om_source_ingress"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_source_ingress/reply"
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-reply-ingress")
    );
    assert!(requests[1].body.contains("\"msg_type\":\"text\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"reply from ingress\\\"")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_prefers_configured_account_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-configured-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-configured"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_source_configured/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_configured_1",
                                "root_id": "om_source_configured",
                                "parent_id": "om_source_configured"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant_for_account(
        &sqlite_path,
        "feishu_shared",
        "ou_shared",
        "u-token-reply-configured",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "text": "reply from configured ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "configured_account_id": "work",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_configured_reply"
                        },
                        "delivery": {
                            "source_message_id": "om_source_configured"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should use configured account from ingress");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["configured_account"], "work");
    assert_eq!(
        outcome.payload["delivery"]["reply_to_message_id"],
        "om_source_configured"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_source_configured/reply"
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-reply-configured")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_reply_tool_falls_back_to_parent_message_id_from_internal_ingress() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("messages-reply-parent-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-parent"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_ingress/reply",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_parent_1",
                                "root_id": "om_parent_ingress",
                                "parent_id": "om_parent_ingress"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-reply-parent",
        &["offline_access", "im:message:send_as_bot"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.reply".to_owned(),
            payload: serde_json::json!({
                "text": "reply from parent fallback",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "conversation_id": "oc_demo"
                        },
                        "delivery": {
                            "parent_message_id": "om_parent_ingress"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu messages reply tool should fall back to parent message id");

    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["delivery"]["reply_to_message_id"],
        "om_parent_ingress"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_ingress/reply"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_requires_callback_token_without_internal_context() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-missing-token");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "done"
                    }]
                }
            }),
        },
        &config,
    )
    .expect_err("missing callback token should fail");

    assert!(error.contains("feishu.card.update requires payload.callback_token"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_requires_card_or_markdown_payload() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-missing-card-and-markdown");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "callback_token": "callback-token-1"
            }),
        },
        &config,
    )
    .expect_err("missing card and markdown should fail");

    assert!(error.contains("feishu.card.update requires payload.card or payload.markdown"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_rejects_card_and_markdown_together() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-card-and-markdown-conflict");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "callback_token": "callback-token-1",
                "markdown": "approved",
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "approved"
                    }]
                }
            }),
        },
        &config,
    )
    .expect_err("card and markdown together should fail");

    assert!(
        error
            .contains("feishu.card.update accepts exactly one of payload.card or payload.markdown")
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_defers_when_callback_context_requests_post_callback_dispatch() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-deferred");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "approved"
                    }]
                },
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_main",
                            "conversation_id": "oc_card_callback"
                        }
                    },
                    "feishu_callback": {
                        "callback_token": "callback-token-deferred",
                        "operator_open_id": "ou_card_operator",
                        "deferred_context_id": "evt_card_deferred_1"
                    }
                }
            }),
        },
        &config,
    )
    .expect("callback-scoped card update should queue deferred work");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["update"]["mode"], "deferred");
    assert_eq!(
        outcome.payload["update"]["open_ids"],
        serde_json::json!(["ou_card_operator"])
    );

    let drained = drain_deferred_feishu_card_updates("evt_card_deferred_1");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].token, "callback-token-deferred");
    assert_eq!(drained[0].open_ids, vec!["ou_card_operator"]);
    assert_eq!(
        drained[0].card,
        serde_json::json!({
            "elements": [{
                "tag": "markdown",
                "content": "approved"
            }]
        })
    );
    assert!(drain_deferred_feishu_card_updates("evt_card_deferred_1").is_empty());
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_rejects_more_than_two_deferred_updates_per_callback_context() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-deferred-limit");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let build_request = || loong_contracts::ToolCoreRequest {
        tool_name: "feishu.card.update".to_owned(),
        payload: serde_json::json!({
            "card": {
                "elements": [{
                    "tag": "markdown",
                    "content": "approved"
                }]
            },
            "_loong": {
                "ingress": {
                    "source": "channel",
                    "channel": {
                        "platform": "feishu",
                        "account_id": "feishu_main",
                        "conversation_id": "oc_card_callback"
                    }
                },
                "feishu_callback": {
                    "callback_token": "callback-token-deferred-limit",
                    "operator_open_id": "ou_card_operator",
                    "deferred_context_id": "evt_card_deferred_limit_1"
                }
            }
        }),
    };

    let first = execute_tool_core_with_test_context(build_request(), &config)
        .expect("first callback-scoped card update should queue deferred work");
    assert_eq!(first.payload["update"]["callback_token_use_count"], 1);
    assert_eq!(first.payload["update"]["callback_token_use_limit"], 2);

    let second = execute_tool_core_with_test_context(build_request(), &config)
        .expect("second callback-scoped card update should stay within token budget");
    assert_eq!(second.payload["update"]["callback_token_use_count"], 2);
    assert_eq!(second.payload["update"]["callback_token_use_limit"], 2);

    let error = execute_tool_core_with_test_context(build_request(), &config)
        .expect_err("third callback-scoped card update should exceed token budget");
    assert!(error.contains("callback token can only be used twice"));

    let drained = drain_deferred_feishu_card_updates("evt_card_deferred_limit_1");
    assert_eq!(drained.len(), 2);
    assert!(drain_deferred_feishu_card_updates("evt_card_deferred_limit_1").is_empty());
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_card_update_tool_accepts_markdown_shortcut() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("card-update-markdown-shortcut");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-card-update-markdown"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "callback_token": "callback-token-markdown",
                "shared": true,
                "markdown": "Approved for everyone"
            }),
        },
        &config,
    )
    .expect("markdown shortcut should build a standard card");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["update"]["shared"], serde_json::json!(true));

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
    assert!(
        requests[1]
            .body
            .contains("\"token\":\"callback-token-markdown\"")
    );
    assert!(requests[1].body.contains("\"wide_screen_mode\":true"));
    assert!(
        requests[1]
            .body
            .contains("\"content\":\"Approved for everyone\"")
    );
    assert!(
        !requests[1].body.contains("\"open_ids\""),
        "shared markdown card update should not send open_ids"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_card_update_tool_defaults_account_callback_token_and_open_ids_from_internal_context()
 {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("card-update-ingress-defaults");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-card-update"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "approved"
                    }]
                },
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_main",
                            "conversation_id": "oc_card_callback"
                        }
                    },
                    "feishu_callback": {
                        "callback_token": "callback-token-from-ingress",
                        "open_message_id": "om_card_callback_1",
                        "open_chat_id": "oc_card_callback",
                        "operator_open_id": "ou_card_operator"
                    }
                }
            }),
        },
        &config,
    )
    .expect("feishu card update tool should default from internal callback context");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["account_id"], "feishu_main");
    assert_eq!(outcome.payload["update"]["message"], "ok");
    assert_eq!(
        outcome.payload["update"]["open_ids"],
        serde_json::json!(["ou_card_operator"])
    );
    assert_eq!(
        outcome.payload["update"]["callback_open_message_id"],
        "om_card_callback_1"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-card-update")
    );
    assert!(
        requests[1]
            .body
            .contains("\"token\":\"callback-token-from-ingress\"")
    );
    assert!(
        requests[1]
            .body
            .contains("\"open_ids\":[\"ou_card_operator\"]")
    );
    assert!(requests[1].body.contains("\"content\":\"approved\""));

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_card_update_tool_explicit_callback_token_and_open_ids_override_internal_defaults() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("card-update-explicit-overrides");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-card-update-explicit"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "account_id": "feishu_main",
                "callback_token": "callback-token-explicit",
                "open_ids": [],
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "shared update"
                    }]
                },
                "_loong": {
                    "feishu_callback": {
                        "callback_token": "callback-token-from-ingress",
                        "operator_open_id": "ou_card_operator"
                    }
                }
            }),
        },
        &config,
    )
    .expect("explicit payload should override internal callback defaults");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["update"]["open_ids"], serde_json::json!([]));

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
    assert!(
        requests[1]
            .body
            .contains("\"token\":\"callback-token-explicit\"")
    );
    assert!(
        !requests[1].body.contains("callback-token-from-ingress"),
        "explicit callback token should override internal token"
    );
    assert!(
        !requests[1].body.contains("\"open_ids\""),
        "explicit empty open_ids should suppress exclusive-card defaults"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_card_update_tool_shared_flag_suppresses_callback_open_id_default() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("card-update-shared-flag");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "tenant_access_token": "t-token-card-update-shared"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "shared": true,
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "shared update explicit"
                    }]
                },
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_main",
                            "conversation_id": "oc_card_callback"
                        }
                    },
                    "feishu_callback": {
                        "callback_token": "callback-token-shared",
                        "operator_open_id": "ou_card_operator"
                    }
                }
            }),
        },
        &config,
    )
    .expect("shared card update should suppress callback operator default");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["update"]["open_ids"], serde_json::json!([]));
    assert_eq!(outcome.payload["update"]["shared"], serde_json::json!(true));

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
    assert!(
        !requests[1].body.contains("\"open_ids\""),
        "shared card update should not send callback operator defaults"
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_card_update_tool_rejects_nonempty_open_ids_when_shared() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("card-update-shared-open-ids-conflict");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "callback_token": "callback-token-shared-conflict",
                "shared": true,
                "open_ids": ["ou_card_operator"],
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "shared conflict"
                    }]
                }
            }),
        },
        &config,
    )
    .expect_err("shared card update should reject non-empty open_ids");

    assert!(
        error.contains("payload.shared=true cannot be combined with non-empty payload.open_ids")
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_direct_tool_execution_rejects_reserved_internal_payload() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("reserved-internal-payload");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = super::execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "text": "ship by ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_main",
                            "conversation_id": "oc_ingress_send"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("direct execution should reject reserved internal payloads");

    assert!(
        error.contains("payload._loong is reserved for trusted internal tool context"),
        "error={error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_messages_send_tool_ignores_non_feishu_internal_ingress_context() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-non-feishu-ingress");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let _store = seed_feishu_tool_grant(&sqlite_path, "u-token-send-plain", &["offline_access"]);
    let config = build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "text": "ship by ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "telegram",
                            "conversation_id": "chat_telegram_1"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("non-feishu ingress should not satisfy receive_id defaults");

    assert!(
        error.contains("feishu.messages.send requires payload.receive_id"),
        "error={error}"
    );
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[test]
fn feishu_messages_send_tool_reports_ambiguous_runtime_account_from_internal_ingress() {
    use std::fs;

    let temp_dir = unique_feishu_tool_temp_dir("messages-send-ambiguous-runtime-account");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let config = runtime_config::ToolRuntimeConfig {
        feishu: Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                accounts: BTreeMap::from([
                    (
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline("cli_work".to_owned())),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-work".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                    (
                        "alerts".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some(loong_contracts::SecretRef::Inline(
                                "cli_alerts".to_owned(),
                            )),
                            app_secret: Some(loong_contracts::SecretRef::Inline(
                                "app-secret-alerts".to_owned(),
                            )),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    ),
                ]),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig {
                sqlite_path: sqlite_path.display().to_string(),
                ..crate::config::FeishuIntegrationConfig::default()
            },
        }),
        ..runtime_config::ToolRuntimeConfig::default()
    };

    let error = execute_tool_core_with_test_context(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.messages.send".to_owned(),
            payload: serde_json::json!({
                "text": "ship by ingress",
                "_loong": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_shared",
                            "conversation_id": "oc_ingress_send"
                        }
                    }
                }
            }),
        },
        &config,
    )
    .expect_err("ambiguous runtime account should fail");

    assert!(error.contains("requested Feishu runtime account `feishu_shared` is ambiguous"));
    assert!(error.contains("Use configured_account_id `alerts` or `work` to disambiguate"));
    assert!(error.contains("work"));
    assert!(error.contains("alerts"));
    assert!(error.contains("payload.account_id"));
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_calendar_freebusy_tool_defaults_selected_open_id_and_user_token() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("calendar-freebusy");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/calendar/v4/freebusy/list",
        post({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "freebusy_list": [{
                                "start_time": "2026-03-12T09:00:00+08:00",
                                "end_time": "2026-03-12T10:00:00+08:00",
                                "rsvp_status": "busy"
                            }]
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-freebusy",
        &["offline_access", "calendar:calendar:readonly"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.calendar.freebusy".to_owned(),
            payload: serde_json::json!({
                "time_min": "2026-03-12T09:00:00+08:00",
                "time_max": "2026-03-12T10:00:00+08:00",
                "include_external_calendar": true,
                "only_busy": true,
                "need_rsvp_status": true
            }),
        },
        &config,
    )
    .expect("feishu calendar freebusy tool should succeed");

    assert_eq!(
        outcome.payload["result"]["freebusy_list"][0]["rsvp_status"],
        "busy"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-freebusy")
    );
    assert!(
        requests[0]
            .query
            .as_deref()
            .is_some_and(|query| { query.contains("user_id_type=open_id") })
    );
    assert!(requests[0].body.contains("\"user_id\":\"ou_123\""));
    assert!(
        requests[0]
            .body
            .contains("\"include_external_calendar\":true")
    );

    server.abort();
}

#[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_calendar_list_tool_primary_defaults_open_id_and_user_token() {
    use std::fs;

    use axum::{
        Json, Router,
        extract::{Request, State},
        routing::post,
    };

    let temp_dir = unique_feishu_tool_temp_dir("calendar-list");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let sqlite_path = temp_dir.join("feishu.sqlite3");
    let requests =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
    let state = FeishuToolMockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/calendar/v4/calendars/primary",
        post({
            let state = state.clone();
            move |request: Request| {
                let state = state.clone();
                async move {
                    record_feishu_tool_request(State(state), request).await;
                    Json(serde_json::json!({
                        "code": 0,
                        "data": {
                            "calendars": [{
                                "calendar": {
                                    "calendar_id": "cal_primary",
                                    "summary": "Alice Primary",
                                    "permissions": "private"
                                },
                                "user_id": "ou_123"
                            }]
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
    let _store = seed_feishu_tool_grant(
        &sqlite_path,
        "u-token-calendar",
        &["offline_access", "calendar:calendar:readonly"],
    );
    let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

    let outcome = execute_tool_core_with_config(
        loong_contracts::ToolCoreRequest {
            tool_name: "feishu.calendar.list".to_owned(),
            payload: serde_json::json!({
                "primary": true
            }),
        },
        &config,
    )
    .expect("feishu calendar list tool should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["primary"], true);
    assert_eq!(
        outcome.payload["calendars"]["calendars"][0]["calendar"]["calendar_id"],
        "cal_primary"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-calendar")
    );
    assert_eq!(requests[0].path, "/open-apis/calendar/v4/calendars/primary");
    let query = requests[0].query.as_deref().unwrap_or_default();
    assert!(query.contains("user_id_type=open_id"));

    server.abort();
}

#[test]
fn provider_switch_tool_updates_target_config_and_reports_active_profile() {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    let root = unique_temp_dir("loongclaw-tool-provider-switch");
    fs::create_dir_all(&root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");

    let mut config = crate::config::LoongConfig::default();
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    config.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai.clone(),
        },
    );
    let mut deepseek =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
    deepseek.model = "deepseek-chat".to_owned();
    config.providers.insert(
        "deepseek-chat".to_owned(),
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: deepseek,
        },
    );
    config.provider = openai;
    config.active_provider = Some("openai-gpt-5".to_owned());
    fs::write(
        &config_path,
        crate::config::render(&config).expect("render provider config"),
    )
    .expect("write provider config");

    let runtime_config = runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::new(),
        file_root: Some(root.clone()),
        config_path: Some(config_path.clone()),
        external_skills: Default::default(),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "provider.switch".to_owned(),
            payload: json!({
                "selector": "deepseek",
                "config_path": "loongclaw.toml"
            }),
        },
        &runtime_config,
    )
    .expect("provider switch should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], "provider.switch");
    assert_eq!(outcome.payload["changed"], true);
    assert_eq!(outcome.payload["previous_active_provider"], "openai-gpt-5");
    assert_eq!(outcome.payload["active_provider"], "deepseek-chat");

    let (_, reloaded) =
        crate::config::load(Some(config_path.to_str().expect("utf8 config path"))).expect("load");
    assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn provider_switch_tool_accepts_unique_model_selector() {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    let root = unique_temp_dir("loongclaw-tool-provider-switch-model");
    fs::create_dir_all(&root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");

    let mut config = crate::config::LoongConfig::default();
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    config.set_active_provider_profile(
        "openai-main",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai.clone(),
        },
    );
    let mut deepseek =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
    deepseek.model = "deepseek-chat".to_owned();
    config.providers.insert(
        "deepseek-cn".to_owned(),
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: deepseek,
        },
    );
    config.provider = openai;
    config.active_provider = Some("openai-main".to_owned());
    fs::write(
        &config_path,
        crate::config::render(&config).expect("render provider config"),
    )
    .expect("write provider config");

    let runtime_config = runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::new(),
        file_root: Some(root.clone()),
        config_path: Some(config_path.clone()),
        external_skills: Default::default(),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "provider.switch".to_owned(),
            payload: json!({
                "selector": "deepseek-chat"
            }),
        },
        &runtime_config,
    )
    .expect("provider switch by model should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["changed"], true);
    assert_eq!(outcome.payload["previous_active_provider"], "openai-main");
    assert_eq!(outcome.payload["active_provider"], "deepseek-cn");

    let (_, reloaded) =
        crate::config::load(Some(config_path.to_str().expect("utf8 config path"))).expect("load");
    assert_eq!(reloaded.active_provider_id(), Some("deepseek-cn"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn provider_switch_without_selector_reports_current_provider_state() {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    let root = unique_temp_dir("loongclaw-tool-provider-switch-inspect");
    fs::create_dir_all(&root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");

    let mut config = crate::config::LoongConfig::default();
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    config.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai,
        },
    );
    fs::write(
        &config_path,
        crate::config::render(&config).expect("render provider config"),
    )
    .expect("write provider config");

    let runtime_config = runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::new(),
        file_root: Some(root.clone()),
        config_path: Some(config_path.clone()),
        external_skills: Default::default(),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "provider.switch".to_owned(),
            payload: json!({}),
        },
        &runtime_config,
    )
    .expect("provider switch inspect should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["changed"], false);
    assert_eq!(outcome.payload["active_provider"], "openai-gpt-5");
    assert_eq!(outcome.payload["selector"], Value::Null);
    assert_eq!(outcome.payload["profiles"][0]["profile_id"], "openai-gpt-5");
    assert_eq!(
        outcome.payload["profiles"][0]["accepted_selectors"],
        json!(["openai-gpt-5", "gpt-5", "openai"])
    );

    let (_, reloaded) =
        crate::config::load(Some(config_path.to_str().expect("utf8 config path"))).expect("load");
    assert_eq!(reloaded.active_provider_id(), Some("openai-gpt-5"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn unknown_tool_returns_hard_error_code() {
    let err = execute_tool_core(ToolCoreRequest {
        tool_name: "unknown".to_owned(),
        payload: json!({"hello":"world"}),
    })
    .expect_err("unknown tool should return an error");
    assert!(
        err.contains("tool_not_found"),
        "error should contain tool_not_found, got: {err}"
    );
}

#[test]
fn config_import_plan_mode_returns_nativeized_preview() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-plan");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(
        &root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    write_file(
        &root,
        "IDENTITY.md",
        "# Identity\n\n- Motto: your nanobot agent for deploys\n",
    );

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "plan",
                "source": "nanobot",
                "input_path": "."
            }),
        },
        &config,
    )
    .expect("config import plan should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], "config.import");
    assert_eq!(outcome.payload["mode"], "plan");
    assert_eq!(outcome.payload["source"], "nanobot");
    assert_eq!(
        outcome.payload["config_preview"]["prompt_pack_id"],
        "loong-core-v1"
    );
    assert_eq!(
        outcome.payload["config_preview"]["memory_profile"],
        "profile_plus_window"
    );
    assert!(
        outcome.payload["config_preview"]["system_prompt_addendum"]
            .as_str()
            .expect("prompt addendum should exist")
            .contains("Loong")
    );
    assert!(
        outcome.payload["config_preview"]["profile_note"]
            .as_str()
            .expect("profile note should exist")
            .contains("Loong")
    );
    assert_eq!(outcome.payload["config_written"], false);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_apply_mode_writes_target_config() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-apply");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(
        &root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    write_file(
        &root,
        "IDENTITY.md",
        "# Identity\n\n- Motto: your nanobot agent for deploys\n",
    );

    let output_path = root.join("generated").join("loongclaw.toml");
    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let tool_names = ["claw_migrate", "claw.migrate"];

    for tool_name in tool_names {
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: tool_name.to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "nanobot",
                    "input_path": ".",
                    "output_path": "generated/loongclaw.toml",
                    "force": true
                }),
            },
            &config,
        )
        .expect("config import apply should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply");
        assert_eq!(outcome.payload["config_written"], true);
        assert_eq!(
            outcome.payload["next_step"]
                .as_str()
                .expect("next_step should be present")
                .split_whitespace()
                .next(),
            Some("loong")
        );
        assert_eq!(
            outcome.payload["output_path"]
                .as_str()
                .expect("output path should exist"),
            dunce::canonicalize(&output_path)
                .expect("output path should canonicalize")
                .display()
                .to_string()
        );
    }

    let raw = fs::read_to_string(&output_path).expect("output config should exist");
    assert!(raw.contains("prompt_pack_id = \"loong-core-v1\""));
    assert!(raw.contains("profile = \"profile_plus_window\""));
    assert!(raw.contains("Loong"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_discover_mode_returns_detected_sources() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-discover");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
    );

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "discover",
                "input_path": "."
            }),
        },
        &config,
    )
    .expect("config import discover should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "discover");
    assert_eq!(outcome.payload["sources"][0]["source_id"], "openclaw");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_plan_many_mode_returns_source_summaries_and_recommendation() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-plan-many");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
    );

    let nanobot_root = root.join("nanobot");
    fs::create_dir_all(&nanobot_root).expect("create nanobot root");
    write_file(
        &nanobot_root,
        "IDENTITY.md",
        "# Identity\n\n- Motto: your nanobot agent for deploys\n",
    );

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "plan_many",
                "input_path": "."
            }),
        },
        &config,
    )
    .expect("config import plan_many should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "plan_many");
    assert_eq!(outcome.payload["plans"][0]["source_id"], "openclaw");
    assert_eq!(outcome.payload["recommendation"]["source_id"], "openclaw");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_merge_profiles_mode_preserves_prompt_owner() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-merge-profiles");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );

    let nanobot_root = root.join("nanobot");
    fs::create_dir_all(&nanobot_root).expect("create nanobot root");
    write_file(
        &nanobot_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- region: apac\n",
    );

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "merge_profiles",
                "input_path": "."
            }),
        },
        &config,
    )
    .expect("config import merge_profiles should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "merge_profiles");
    assert_eq!(
        outcome.payload["result"]["prompt_owner_source_id"],
        "openclaw"
    );
    assert!(
        outcome.payload["result"]["merged_profile_note"]
            .as_str()
            .expect("merged profile note should be present")
            .contains("region: apac")
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_map_external_skills_mode_returns_mapping_plan() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-map-external-skills");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
    fs::create_dir_all(root.join(".codex/skills")).expect("create codex skills dir");

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "map_external_skills",
                "input_path": "."
            }),
        },
        &config,
    )
    .expect("config import map_external_skills should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "map_external_skills");
    assert_eq!(outcome.payload["result"]["artifact_count"], 2);
    assert_eq!(
        outcome.payload["result"]["declared_skills"][0],
        "custom/skill-a"
    );
    assert_eq!(
        outcome.payload["result"]["resolved_skills"][0],
        "custom/skill-a"
    );
    assert!(
        outcome.payload["result"]["profile_note_addendum"]
            .as_str()
            .expect("profile note addendum should exist")
            .contains("Imported External Skills Artifacts")
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_apply_selected_mode_writes_manifest_and_backup() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-apply-selected");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );

    let output_path = root.join("loongclaw.toml");
    let original_body = crate::config::render(&crate::config::LoongConfig::default())
        .expect("render default config");
    fs::write(&output_path, &original_body).expect("write original config");

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "apply_selected",
                "input_path": ".",
                "output_path": "loongclaw.toml",
                "source_id": "openclaw"
            }),
        },
        &config,
    )
    .expect("config import apply_selected should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "apply_selected");
    assert!(
        Path::new(
            outcome.payload["result"]["backup_path"]
                .as_str()
                .expect("backup path should be present")
        )
        .exists()
    );
    assert!(
        Path::new(
            outcome.payload["result"]["manifest_path"]
                .as_str()
                .expect("manifest path should be present")
        )
        .exists()
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_apply_selected_mode_can_apply_external_skills_plan() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-apply-selected-external");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );
    write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
    write_file(
        &root,
        ".codex/skills/release-guard/SKILL.md",
        "# Release Guard\n\nUse this skill when release discipline matters.\n",
    );

    let output_path = root.join("loongclaw.toml");

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "apply_selected",
                "input_path": ".",
                "output_path": "loongclaw.toml",
                "source_id": "openclaw",
                "apply_external_skills_plan": true
            }),
        },
        &config,
    )
    .expect("config import apply_selected with external skills should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["result"]["external_skill_artifact_count"],
        2
    );
    assert_eq!(
        outcome.payload["result"]["external_skill_entries_applied"],
        6
    );
    assert_eq!(
        outcome.payload["result"]["external_skill_managed_install_count"],
        1
    );
    assert_eq!(
        outcome.payload["result"]["external_skill_managed_skill_ids"],
        json!(["release-guard"])
    );
    assert!(
        outcome.payload["result"]["external_skills_manifest_path"]
            .as_str()
            .is_some(),
        "external skills manifest path should exist"
    );
    let raw = fs::read_to_string(&output_path).expect("read output config");
    assert!(raw.contains("Imported External Skills Artifacts"));
    assert!(
        root.join("external-skills-installed")
            .join("release-guard")
            .join("SKILL.md")
            .exists(),
        "config.import should bridge installable local skills into the managed runtime"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn config_import_rollback_last_apply_restores_original_config() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-tool-import-rollback-selected");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );

    let output_path = root.join("loongclaw.toml");
    let original_body = crate::config::render(&crate::config::LoongConfig::default())
        .expect("render default config");
    fs::write(&output_path, &original_body).expect("write original config");

    let config = runtime_config::ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..runtime_config::ToolRuntimeConfig::default()
    };
    execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "apply_selected",
                "input_path": ".",
                "output_path": "loongclaw.toml",
                "source_id": "openclaw"
            }),
        },
        &config,
    )
    .expect("config import apply_selected should succeed");

    let rollback = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: json!({
                "mode": "rollback_last_apply",
                "output_path": "loongclaw.toml"
            }),
        },
        &config,
    )
    .expect("config import rollback_last_apply should succeed");

    assert_eq!(rollback.status, "ok");
    assert!(
        rollback.payload["rolled_back"]
            .as_bool()
            .expect("rolled_back flag should exist")
    );
    assert_eq!(
        fs::read_to_string(&output_path).expect("read restored config"),
        original_body
    );

    fs::remove_dir_all(&root).ok();
}

// --- Kernel-routed tool tests ---

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use loong_contracts::{ExecutionRoute, HarnessKind, ToolPlaneError};
use loong_kernel::{
    CoreToolAdapter, FixedClock, InMemoryAuditSink, LoongKernel, StaticPolicyEngine,
    VerticalPackManifest,
};

struct SharedTestToolAdapter {
    invocations: Arc<Mutex<Vec<ToolCoreRequest>>>,
}

#[async_trait]
impl CoreToolAdapter for SharedTestToolAdapter {
    fn name(&self) -> &str {
        "test-tool-shared"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request);
        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({}),
        })
    }
}

fn build_tool_kernel_context(
    audit: Arc<InMemoryAuditSink>,
    capabilities: BTreeSet<Capability>,
) -> (KernelContext, Arc<Mutex<Vec<ToolCoreRequest>>>) {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: capabilities,
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");

    let invocations = Arc::new(Mutex::new(Vec::new()));
    let adapter = SharedTestToolAdapter {
        invocations: invocations.clone(),
    };
    kernel.register_core_tool_adapter(adapter);
    kernel
        .set_default_core_tool_adapter("test-tool-shared")
        .expect("set default tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    (ctx, invocations)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_call_through_kernel_records_audit() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) =
        build_tool_kernel_context(audit.clone(), BTreeSet::from([Capability::InvokeTool]));

    let request = ToolCoreRequest {
        tool_name: "echo".to_owned(),
        payload: json!({"msg": "hello"}),
    };
    let outcome = execute_tool(request, &ctx)
        .await
        .expect("tool call via kernel should succeed");
    assert_eq!(outcome.status, "ok");

    // Verify the tool adapter received the request.
    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].tool_name, "echo");

    // Verify audit events contain a tool plane invocation.
    let events = audit.snapshot();
    let has_tool_plane = events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                plane: loong_contracts::ExecutionPlane::Tool,
                ..
            }
        )
    });
    assert!(has_tool_plane, "audit should contain tool plane invocation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mvp_tool_adapter_routes_through_kernel() {
    use kernel_adapter::MvpToolAdapter;

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(MvpToolAdapter::new());
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let caps = BTreeSet::from([Capability::InvokeTool]);
    // Use an unknown tool name — it should propagate as an error through the adapter
    let request = ToolCoreRequest {
        tool_name: "noop".to_owned(),
        payload: json!({"key": "value"}),
    };
    let err = kernel
        .execute_tool_core("test-pack", &token, &caps, None, request)
        .await
        .expect_err("unknown tool via MvpToolAdapter should fail");
    assert!(
        format!("{err}").contains("tool_not_found"),
        "error should contain tool_not_found, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mvp_tool_adapter_rejects_reserved_internal_payload_through_kernel_by_default() {
    use kernel_adapter::MvpToolAdapter;

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(MvpToolAdapter::new());
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let caps = BTreeSet::from([Capability::InvokeTool]);
    let err = kernel
        .execute_tool_core(
            "test-pack",
            &token,
            &caps,
            None,
            ToolCoreRequest {
                tool_name: "shell.exec".to_owned(),
                payload: json!({
                    "command": "echo",
                    "args": ["hello"],
                    "_loong": {
                        "ingress": {
                            "channel": {
                                "platform": "feishu",
                                "conversation_id": "oc_forged"
                            }
                        }
                    }
                }),
            },
        )
        .await
        .expect_err("kernel-routed tool call should reject reserved internal payload by default");

    assert!(
        format!("{err}").contains("payload._loong is reserved for trusted internal tool context"),
        "error should reject reserved internal payload, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_call_through_kernel_denied_without_capability() {
    let audit = Arc::new(InMemoryAuditSink::default());
    // Grant MemoryRead only — InvokeTool is missing.
    let (ctx, _invocations) =
        build_tool_kernel_context(audit, BTreeSet::from([Capability::MemoryRead]));

    let request = ToolCoreRequest {
        tool_name: "echo".to_owned(),
        payload: json!({"msg": "hello"}),
    };
    let err = execute_tool(request, &ctx)
        .await
        .expect_err("should be denied without InvokeTool capability");

    // The error message should indicate a policy/capability denial.
    assert!(
        err.contains("denied") || err.contains("capability") || err.contains("Capability"),
        "error should mention denial or capability, got: {err}"
    );
}

#[cfg(feature = "tool-webfetch")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_fetch_through_kernel_requires_network_egress_capability() {
    use kernel_adapter::MvpToolAdapter;

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");

    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.web_fetch.enabled = true;
    kernel.register_core_tool_adapter(MvpToolAdapter::with_config(config));
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .expect("set default");

    let mut token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let removed_network_egress = token
        .allowed_capabilities
        .remove(&Capability::NetworkEgress);
    assert!(
        removed_network_egress,
        "issued token should include network egress before we remove it for the test"
    );

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };
    let request = ToolCoreRequest {
        tool_name: "web.fetch".to_owned(),
        payload: json!({"url": "https://example.com"}),
    };

    let error = execute_kernel_tool_request(&ctx, request, false)
        .await
        .expect_err("web.fetch should fail closed without network egress capability");

    assert!(matches!(
        error,
        loong_kernel::KernelError::Policy(
            loong_kernel::PolicyError::MissingCapability { capability, .. }
        ) if capability == Capability::NetworkEgress
    ));
}

#[cfg(feature = "tool-webfetch")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_fetch_through_kernel_exposes_network_egress_to_policy_extensions() {
    use kernel_adapter::MvpToolAdapter;

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_policy_extension(loong_kernel::test_support::NoNetworkEgressPolicyExtension);

    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.web_fetch.enabled = true;
    kernel.register_core_tool_adapter(MvpToolAdapter::with_config(config));
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };
    let request = ToolCoreRequest {
        tool_name: "web.fetch".to_owned(),
        payload: json!({"url": "https://example.com"}),
    };

    let error = execute_kernel_tool_request(&ctx, request, false)
        .await
        .expect_err("policy extension should block web.fetch network egress");

    assert!(matches!(
        error,
        loong_kernel::KernelError::Policy(
            loong_kernel::PolicyError::ExtensionDenied { ref extension, .. }
        ) if extension == "no-network-egress"
    ));
}
