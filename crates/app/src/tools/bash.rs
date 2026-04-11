use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
#[cfg(feature = "tool-shell")]
use std::time::Instant;

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};

#[cfg(feature = "tool-shell")]
use super::bash_governance::{FinalGovernanceDecision, evaluate_bash_command};
#[cfg(feature = "tool-shell")]
use super::process_exec;
use super::runtime_config::BashExecRuntimePolicy;
#[cfg(feature = "tool-shell")]
use super::runtime_events::current_tool_runtime_event_sink;

const BASH_UNAVAILABLE_WARNING: &str =
    "bash unavailable; hiding bash.exec from runtime tool surface";
const BASH_RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(feature = "tool-shell")]
const BASH_EXEC_ALLOWED_FIELDS: &[&str] = &[
    "command",
    "cwd",
    "timeout_ms",
    super::LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY,
];

pub(super) fn unavailable_bash_runtime_policy() -> BashExecRuntimePolicy {
    BashExecRuntimePolicy {
        warning: Some(BASH_UNAVAILABLE_WARNING.to_owned()),
        ..BashExecRuntimePolicy::default()
    }
}

pub(super) fn bash_runtime_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from("bash")];

    if cfg!(windows) {
        candidates.extend([
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"),
            PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
            PathBuf::from(r"C:\msys64\usr\bin\bash.exe"),
            PathBuf::from(r"C:\cygwin64\bin\bash.exe"),
            PathBuf::from(r"C:\cygwin\bin\bash.exe"),
        ]);
    }

    candidates
}

#[allow(dead_code)]
pub(super) fn bash_exec_args(command: &str, login_shell: bool) -> Vec<String> {
    if login_shell {
        vec!["-lc".to_owned(), command.to_owned()]
    } else {
        vec!["-c".to_owned(), command.to_owned()]
    }
}

pub(super) fn detect_bash_runtime_policy() -> BashExecRuntimePolicy {
    for candidate in bash_runtime_candidates() {
        if probe_bash_candidate(&candidate) {
            return BashExecRuntimePolicy {
                available: true,
                command: Some(candidate),
                ..BashExecRuntimePolicy::default()
            };
        }
    }

    unavailable_bash_runtime_policy()
}

pub(super) fn execute_bash_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-shell"))]
    {
        let _ = (request, config);
        return Err("bash tool is disabled in this build (enable feature `tool-shell`)".to_owned());
    }

    #[cfg(feature = "tool-shell")]
    {
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| "bash.exec payload must be an object".to_owned())?;
        reject_unknown_bash_exec_fields(payload)?;
        let command = payload
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "bash.exec requires payload.command".to_owned())?;
        let cwd = parse_bash_cwd(payload, config)?;
        let timeout_ms = parse_bash_timeout_ms(payload)?;

        if !config.bash_exec.is_runtime_ready() {
            return Err("bash unavailable".to_owned());
        }

        let runtime = &config.bash_exec;
        if let Some(load_error) = runtime.governance.load_error.as_deref() {
            return Err(format!(
                "policy_denied: bash governance rules failed to load: {load_error}"
            ));
        }

        let governance = evaluate_bash_command(
            command,
            &runtime.governance.rules,
            config.shell_default_mode,
        );
        if governance.final_decision == FinalGovernanceDecision::Deny {
            let detail = governance
                .denial_reason()
                .unwrap_or_else(|| "bash governance denied command".to_owned());
            return Err(format!("policy_denied: {detail}"));
        }

        let runtime_command = runtime
            .command
            .as_deref()
            .ok_or_else(|| "bash unavailable".to_owned())?;
        let args = bash_exec_args(command, runtime.login_shell);
        let runtime_event_sink = current_tool_runtime_event_sink();
        let output = process_exec::run_tool_async(
            process_exec::run_process_with_timeout_with_sink(
                runtime_command,
                &args,
                cwd.as_path(),
                timeout_ms,
                "bash command",
                runtime_event_sink.clone(),
            ),
            "bash tool",
        )??;

        Ok(ToolCoreOutcome {
            status: if output.status.success() {
                "ok".to_owned()
            } else {
                "failed".to_owned()
            },
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "command": command,
                "cwd": cwd.display().to_string(),
                "exit_code": output.status.code(),
                "stdout": String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                "stderr": String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            }),
        })
    }
}

#[cfg(feature = "tool-shell")]
fn reject_unknown_bash_exec_fields(payload: &serde_json::Map<String, Value>) -> Result<(), String> {
    let mut unknown_fields = payload
        .keys()
        .filter(|field| !BASH_EXEC_ALLOWED_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unknown_fields.sort();

    if unknown_fields.is_empty() {
        return Ok(());
    }

    Err(format!(
        "bash.exec payload contains unknown field(s): {}",
        unknown_fields.join(", ")
    ))
}

#[cfg(feature = "tool-shell")]
fn parse_bash_cwd(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    match payload.get("cwd") {
        Some(cwd) => cwd
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| "bash.exec payload.cwd must be a string".to_owned()),
        None => {
            let current_dir = std::env::current_dir();
            if let Ok(current_dir) = current_dir {
                return Ok(current_dir);
            }

            if let Some(file_root) = config.file_root.clone() {
                return Ok(file_root);
            }

            Ok(PathBuf::from("."))
        }
    }
}

#[cfg(feature = "tool-shell")]
fn parse_bash_timeout_ms(payload: &serde_json::Map<String, Value>) -> Result<u64, String> {
    let timeout_ms = match payload.get("timeout_ms") {
        Some(timeout_ms) => timeout_ms
            .as_u64()
            .ok_or_else(|| "bash.exec payload.timeout_ms must be an integer".to_owned())?,
        None => process_exec::DEFAULT_TIMEOUT_MS,
    };

    Ok(timeout_ms.clamp(1_000, process_exec::MAX_TIMEOUT_MS))
}

fn probe_bash_candidate(candidate: &Path) -> bool {
    probe_bash_candidate_with_timeout(candidate, BASH_RUNTIME_PROBE_TIMEOUT)
}

fn probe_bash_candidate_with_timeout(candidate: &Path, timeout: Duration) -> bool {
    let Ok(mut child) = Command::new(candidate)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    let timeout = timeout.max(Duration::from_millis(1));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                thread::park_timeout(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::bash_rules::{PrefixRuleDecision, compile_compatibility_rules};
    use crate::tools::runtime_config::ToolRuntimeConfig;
    use crate::tools::runtime_events::{
        ToolRuntimeEvent, ToolRuntimeEventSink, ToolRuntimeStream, with_tool_runtime_event_sink,
    };
    use loongclaw_contracts::ToolCoreRequest;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

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

    #[test]
    fn probe_bash_runtime_prefers_path_bash_before_windows_fallbacks() {
        let candidates = bash_runtime_candidates();

        assert_eq!(
            candidates
                .first()
                .map(|candidate| candidate.to_string_lossy().to_string()),
            Some("bash".to_owned())
        );
    }

    #[test]
    fn unavailable_runtime_policy_carries_warning() {
        let policy = unavailable_bash_runtime_policy();

        assert!(!policy.available);
        assert!(policy.command.is_none());
        assert_eq!(policy.warning.as_deref(), Some(BASH_UNAVAILABLE_WARNING));
    }

    #[cfg(unix)]
    #[test]
    fn probe_bash_candidate_with_timeout_returns_false_for_slow_candidate() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let script = tempdir.path().join("slow-bash");
        fs::write(&script, "#!/bin/sh\nsleep 2\nexit 0\n").expect("write slow probe script");
        let mut permissions = fs::metadata(&script)
            .expect("stat slow probe script")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod slow probe script");

        let start = Instant::now();
        let available = probe_bash_candidate_with_timeout(&script, Duration::from_millis(50));

        assert!(!available);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "slow probe should time out quickly, elapsed={:?}",
            start.elapsed()
        );
    }

    #[test]
    fn bash_exec_arg_builder_defaults_to_non_login_shell() {
        let args = bash_exec_args("echo hi", false);

        assert_eq!(args, vec!["-c".to_owned(), "echo hi".to_owned()]);
    }

    #[test]
    fn bash_exec_arg_builder_uses_login_shell_when_enabled() {
        let args = bash_exec_args("echo hi", true);

        assert_eq!(args, vec!["-lc".to_owned(), "echo hi".to_owned()]);
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn execute_bash_tool_with_config_reports_unavailable_runtime() {
        let config = ToolRuntimeConfig::default();
        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "echo hi",
                "cwd": ".",
                "timeout_ms": 1000
            }),
        };

        let error = execute_bash_tool_with_config(request, &config)
            .expect_err("runtime should be required");

        assert!(error.contains("bash unavailable"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn execute_bash_tool_with_config_rejects_non_string_cwd() {
        let config = ToolRuntimeConfig::default();
        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "echo hi",
                "cwd": 123
            }),
        };

        let error =
            execute_bash_tool_with_config(request, &config).expect_err("non-string cwd must fail");

        assert!(error.contains("bash.exec payload.cwd must be a string"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn execute_bash_tool_with_config_rejects_unknown_fields() {
        let config = ToolRuntimeConfig::default();
        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "echo hi",
                "extra": true
            }),
        };

        let error = execute_bash_tool_with_config(request, &config)
            .expect_err("unknown payload fields must fail");

        assert!(error.contains("bash.exec payload contains unknown field(s): extra"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn execute_bash_tool_with_config_allows_trusted_internal_context_field() {
        let config = ToolRuntimeConfig::default();
        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "echo hi",
                "_loongclaw": {
                    "tool_search": {
                        "visible_tool_ids": ["bash.exec"]
                    }
                }
            }),
        };

        let error = execute_bash_tool_with_config(request, &config)
            .expect_err("runtime should still be required after accepting trusted context");

        assert!(error.contains("bash unavailable"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn execute_bash_tool_with_config_emits_runtime_output_delta_and_single_metrics_event() {
        let bash_available = probe_bash_candidate(Path::new("bash"));
        if !bash_available {
            eprintln!("skipping bash runtime event test because bash is unavailable");
            return;
        }

        let root = tempfile::tempdir().expect("tempdir");
        let root_path = root.path().to_path_buf();
        let mut config = ToolRuntimeConfig {
            file_root: Some(root_path.clone()),
            shell_allow: ["printf".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        };
        config.bash_exec.available = true;
        config.bash_exec.command = Some(PathBuf::from("bash"));
        config.bash_exec.governance.rules = compile_compatibility_rules(
            "test_allow",
            PrefixRuleDecision::Allow,
            ["printf", "hello"],
        );

        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "printf 'hello\\nworld'",
                "cwd": root_path.display().to_string(),
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_bash_tool_with_config(request, &config)
        }));
        let outcome = outcome.expect("bash.exec should succeed under runtime sink");
        let events = lock_runtime_events(&sink);
        let stdout_delta_count = events
            .iter()
            .filter(|event| {
                let ToolRuntimeEvent::OutputDelta(delta) = event else {
                    return false;
                };
                let is_stdout = delta.stream == ToolRuntimeStream::Stdout;
                let contains_output = delta.chunk.contains("hello");
                is_stdout && contains_output
            })
            .count();
        let metrics_count = events
            .iter()
            .filter(|event| matches!(event, ToolRuntimeEvent::CommandMetrics(_)))
            .count();

        assert_eq!(outcome.status, "ok");
        assert!(stdout_delta_count >= 1, "events: {events:?}");
        assert_eq!(metrics_count, 1, "events: {events:?}");
    }

    #[cfg(not(feature = "tool-shell"))]
    #[test]
    fn execute_bash_tool_with_config_reports_disabled_feature() {
        let config = ToolRuntimeConfig::default();
        let request = ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "echo hi",
                "cwd": ".",
                "timeout_ms": 1000
            }),
        };

        let error = execute_bash_tool_with_config(request, &config)
            .expect_err("tool-shell-disabled build should fail closed");

        assert!(error.contains("bash tool is disabled in this build"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn parse_bash_timeout_ms_clamps_to_1000ms_floor() {
        let payload = json!({
            "timeout_ms": 1
        });
        let payload = payload.as_object().expect("payload object");

        let timeout_ms = parse_bash_timeout_ms(payload).expect("timeout should parse");

        assert_eq!(timeout_ms, 1_000);
    }
}
