#[cfg(feature = "tool-shell")]
use super::process_exec;
#[cfg(feature = "tool-shell")]
use super::runtime_events::current_tool_runtime_event_sink;
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::Value;
#[cfg(feature = "tool-shell")]
use std::path::PathBuf;
pub(super) fn execute_shell_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-shell"))]
    {
        let _ = (request, config);
        return Err(
            "shell tool is disabled in this build (enable feature `tool-shell`)".to_owned(),
        );
    }

    #[cfg(feature = "tool-shell")]
    {
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| "shell.exec payload must be an object".to_owned())?;
        let command = payload
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "shell.exec requires payload.command".to_owned())?;
        let args = payload
            .get("args")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cwd = resolve_shell_cwd_with_config(payload, config)?;
        let timeout_ms = parse_shell_timeout_ms(payload)?;

        let normalized_command =
            crate::tools::shell_policy_ext::validate_shell_command_name(command)?;
        let basename = normalized_command.as_str();

        if config.shell_deny.contains(basename) {
            return Err(format!(
                "policy_denied: shell command `{basename}` is blocked by shell policy"
            ));
        }

        let explicitly_allowed = config.shell_allow.contains(basename);
        let default_allows = matches!(
            config.shell_default_mode,
            crate::tools::shell_policy_ext::ShellPolicyDefault::Allow
        );
        let approval_key =
            crate::tools::shell_policy_ext::shell_exec_approval_key_for_normalized_command(
                basename,
            );
        let approved_by_internal_context =
            crate::tools::shell_policy_ext::shell_exec_matches_trusted_internal_approval(
                payload,
                approval_key.as_str(),
            );
        if !explicitly_allowed && !default_allows && !approved_by_internal_context {
            return Err(format!(
                "policy_denied: shell command `{basename}` is not in the allow list (default-deny policy)"
            ));
        }

        let runtime_event_sink = current_tool_runtime_event_sink();
        let resolved_invocation = crate::process_launch::resolve_command_invocation(
            normalized_command.as_str(),
            args.iter().map(String::as_str),
        );
        // process_exec owns runtime command metrics emission. Keep shell.exec
        // focused on payload construction so the live surface observes one
        // metrics event per command.
        let output = run_shell_async(run_shell_command_with_timeout(
            resolved_invocation.program.as_os_str(),
            resolved_invocation.args.as_slice(),
            cwd.as_path(),
            timeout_ms,
            runtime_event_sink.clone(),
            config.file_root.as_deref(),
        ))??;

        Ok(process_exec::build_process_tool_outcome(
            request.tool_name.as_str(),
            command,
            Some(args.as_slice()),
            cwd.as_path(),
            output,
        ))
    }
}

#[cfg(feature = "tool-shell")]
fn resolve_shell_cwd_with_config(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    process_exec::resolve_process_cwd_with_config(payload, config, "shell.exec")
}

#[cfg(feature = "tool-shell")]
fn parse_shell_timeout_ms(payload: &serde_json::Map<String, Value>) -> Result<u64, String> {
    let timeout_ms = match payload.get("timeout_ms") {
        Some(timeout_ms) => timeout_ms
            .as_u64()
            .ok_or_else(|| "shell.exec payload.timeout_ms must be an integer".to_owned())?,
        None => process_exec::DEFAULT_TIMEOUT_MS,
    };

    Ok(timeout_ms.clamp(1_000, process_exec::MAX_TIMEOUT_MS))
}

#[cfg(feature = "tool-shell")]
fn run_shell_async<F>(future: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    process_exec::run_tool_async(future, "shell tool")
}

#[cfg(feature = "tool-shell")]
async fn run_shell_command_with_timeout(
    command: &std::ffi::OsStr,
    args: &[std::ffi::OsString],
    cwd: &std::path::Path,
    timeout_ms: u64,
    runtime_event_sink: Option<
        std::sync::Arc<dyn crate::tools::runtime_events::ToolRuntimeEventSink>,
    >,
    accessible_output_root: Option<&std::path::Path>,
) -> Result<process_exec::ProcessExecOutcome, String> {
    process_exec::run_process_with_timeout_with_sink(
        command,
        args,
        cwd,
        timeout_ms,
        "shell command",
        runtime_event_sink,
        accessible_output_root,
    )
    .await
}

#[cfg(all(test, feature = "tool-shell", unix))]
mod tests {
    use super::*;
    use crate::test_support::{ScopedEnv, acquire_subprocess_test_guard, unique_temp_dir};
    use crate::tools::runtime_config::ToolRuntimeConfig;
    use crate::tools::runtime_events::{
        ToolRuntimeEvent, ToolRuntimeEventSink, ToolRuntimeStream, with_tool_runtime_event_sink,
    };
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingRuntimeSink {
        events: Mutex<Vec<ToolRuntimeEvent>>,
    }

    fn lock_runtime_events(
        sink: &RecordingRuntimeSink,
    ) -> std::sync::MutexGuard<'_, Vec<ToolRuntimeEvent>> {
        match sink.events.lock() {
            Ok(events) => events,
            Err(poisoned_events) => poisoned_events.into_inner(),
        }
    }

    impl ToolRuntimeEventSink for RecordingRuntimeSink {
        fn emit(&self, event: ToolRuntimeEvent) {
            let mut events = lock_runtime_events(self);
            events.push(event);
        }
    }

    fn shell_test_config(root: &Path) -> ToolRuntimeConfig {
        ToolRuntimeConfig {
            file_root: Some(root.to_path_buf()),
            shell_allow: ["pwd".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        }
    }

    fn shell_test_env() -> ScopedEnv {
        let mut env = ScopedEnv::new();

        #[cfg(unix)]
        env.set("PATH", "/bin:/usr/bin:/usr/local/bin");

        #[cfg(windows)]
        env.set("PATH", r"C:\Windows\System32;C:\Windows");

        env
    }

    fn execute_shell_tool_for_subprocess_test(
        request: ToolCoreRequest,
        config: &ToolRuntimeConfig,
    ) -> Result<ToolCoreOutcome, String> {
        let _env = shell_test_env();
        let _guard = acquire_subprocess_test_guard();
        execute_shell_tool_with_config(request, config)
    }

    #[test]
    fn shell_exec_defaults_cwd_to_configured_file_root() {
        let root = unique_temp_dir("loong-shell-default-cwd");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd"
            }),
        };

        let outcome = execute_shell_tool_for_subprocess_test(request, &config)
            .expect("shell.exec should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["cwd"], root.display().to_string());
        let stdout = outcome.payload["stdout"]
            .as_str()
            .expect("stdout should be text");
        assert!(
            stdout.ends_with(root.display().to_string().as_str()),
            "expected pwd output to resolve inside file_root, got: {stdout}"
        );
    }

    #[test]
    fn shell_exec_rejects_cwd_that_escapes_configured_file_root() {
        let root = unique_temp_dir("loong-shell-cwd-root");
        let outside = unique_temp_dir("loong-shell-cwd-outside");
        std::fs::create_dir_all(&root).expect("create shell root");
        std::fs::create_dir_all(&outside).expect("create outside dir");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": outside.display().to_string()
            }),
        };

        let error =
            execute_shell_tool_with_config(request, &config).expect_err("escape should fail");

        assert!(
            error.contains("escapes configured file root"),
            "error: {error}"
        );
    }

    #[test]
    fn shell_exec_rejects_non_directory_cwd() {
        let root = unique_temp_dir("loong-shell-cwd-file");
        std::fs::create_dir_all(&root).expect("create shell root");
        let file_path = root.join("note.txt");
        std::fs::write(&file_path, "hello").expect("write shell cwd file");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": "note.txt"
            }),
        };

        let error =
            execute_shell_tool_with_config(request, &config).expect_err("file cwd should fail");

        assert!(error.contains("is not a directory"), "error: {error}");
    }

    #[test]
    fn shell_exec_rejects_non_string_cwd() {
        let root = unique_temp_dir("loong-shell-cwd-non-string");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": 123
            }),
        };

        let error = execute_shell_tool_with_config(request, &config)
            .expect_err("non-string cwd should fail");

        assert!(error.contains("shell.exec payload.cwd must be a string"));
    }

    #[test]
    fn shell_exec_emits_runtime_output_delta_and_metrics_events() {
        let root = unique_temp_dir("loong-shell-runtime-events");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = ToolRuntimeConfig {
            file_root: Some(root),
            shell_allow: ["printf".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "printf",
                "args": ["hello\\nworld"],
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_shell_tool_for_subprocess_test(request, &config)
        }));
        let outcome = outcome.expect("shell.exec should succeed under runtime sink");
        let events = lock_runtime_events(&sink);
        let has_stdout_delta = events.iter().any(|event| {
            if let ToolRuntimeEvent::OutputDelta(delta) = event {
                let is_stdout = delta.stream == ToolRuntimeStream::Stdout;
                let contains_output = delta.chunk.contains("hello");
                return is_stdout && contains_output;
            }

            false
        });
        let has_metrics = events.iter().any(|event| {
            if let ToolRuntimeEvent::CommandMetrics(metrics) = event {
                return metrics.exit_code == Some(0);
            }

            false
        });
        let metrics_count = events
            .iter()
            .filter(|event| matches!(event, ToolRuntimeEvent::CommandMetrics(_)))
            .count();

        assert_eq!(outcome.status, "ok");
        assert!(has_stdout_delta, "events: {events:?}");
        assert!(has_metrics, "events: {events:?}");
        assert_eq!(metrics_count, 1, "events: {events:?}");
    }

    #[test]
    fn shell_exec_runtime_output_delta_counts_terminal_newline_without_extra_line() {
        let root = unique_temp_dir("loong-shell-runtime-line-count");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = ToolRuntimeConfig {
            file_root: Some(root),
            shell_allow: ["printf".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "printf",
                "args": ["alpha\\nbeta\\n"],
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_shell_tool_for_subprocess_test(request, &config)
        }));
        let outcome = outcome.expect("shell.exec should succeed");
        let events = lock_runtime_events(&sink);
        let total_lines = events.iter().rev().find_map(|event| {
            if let ToolRuntimeEvent::OutputDelta(delta) = event {
                let is_stdout = delta.stream == ToolRuntimeStream::Stdout;
                if is_stdout {
                    return Some(delta.total_lines);
                }
            }

            None
        });
        let total_lines = total_lines.expect("stdout delta should exist");

        assert_eq!(outcome.status, "ok");
        assert_eq!(total_lines, 2);
    }

    #[test]
    fn shell_exec_emits_runtime_metrics_for_timeout_failures() {
        let root = unique_temp_dir("loong-shell-runtime-timeout-metrics");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = ToolRuntimeConfig {
            file_root: Some(root),
            shell_allow: ["sh".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "sh",
                "args": ["-c", "sleep 2"],
                "timeout_ms": 1000,
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let error = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_shell_tool_for_subprocess_test(request, &config)
        }));
        let error = error.expect_err("shell.exec should time out");
        let events = lock_runtime_events(&sink);
        let metrics = events.iter().find_map(|event| {
            if let ToolRuntimeEvent::CommandMetrics(metrics) = event {
                return Some(metrics);
            }

            None
        });
        let metrics = metrics.expect("timeout should still emit runtime metrics");
        let metrics_count = events
            .iter()
            .filter(|event| matches!(event, ToolRuntimeEvent::CommandMetrics(_)))
            .count();

        assert!(error.contains("timed out after 1000ms"), "error: {error}");
        assert_eq!(metrics.exit_code, None);
        assert!(metrics.duration_ms > 0, "metrics: {metrics:?}");
        assert_eq!(metrics_count, 1, "events: {events:?}");
    }
}
