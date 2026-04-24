use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wait_timeout::ChildExt;

use crate::process_launch::resolve_command_invocation;
use crate::process_launch::retry_executable_file_busy_with_pause as retry_spawn_with_pause;

const DEFAULT_BROWSER_COMPANION_SCOPE_ID: &str = "__global";
const BROWSER_COMPANION_PROTOCOL: &str = "loong.browser_companion.v1";
const BROWSER_COMPANION_SPAWN_RETRY_ATTEMPTS: usize = 20;
const BROWSER_COMPANION_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
struct BrowserCompanionSession {
    sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserCompanionOperation {
    SessionStart,
    Navigate,
    Snapshot,
    Wait,
    SessionStop,
    Click,
    Type,
}

impl BrowserCompanionOperation {
    fn from_tool_name(tool_name: &str) -> Option<Self> {
        match tool_name {
            "browser.companion.session.start" => Some(Self::SessionStart),
            "browser.companion.navigate" => Some(Self::Navigate),
            "browser.companion.snapshot" => Some(Self::Snapshot),
            "browser.companion.wait" => Some(Self::Wait),
            "browser.companion.session.stop" => Some(Self::SessionStop),
            "browser.companion.click" => Some(Self::Click),
            "browser.companion.type" => Some(Self::Type),
            _ => None,
        }
    }

    fn action_class(self) -> &'static str {
        match self {
            Self::Click | Self::Type => "write",
            Self::SessionStart
            | Self::Navigate
            | Self::Snapshot
            | Self::Wait
            | Self::SessionStop => "read",
        }
    }

    fn is_core(self) -> bool {
        !matches!(self, Self::Click | Self::Type)
    }

    fn is_app(self) -> bool {
        matches!(self, Self::Click | Self::Type)
    }

    fn protocol_name(self) -> &'static str {
        match self {
            Self::SessionStart => "session.start",
            Self::Navigate => "navigate",
            Self::Snapshot => "snapshot",
            Self::Wait => "wait",
            Self::SessionStop => "session.stop",
            Self::Click => "click",
            Self::Type => "type",
        }
    }

    fn requires_existing_session(self) -> bool {
        !matches!(self, Self::SessionStart)
    }
}

#[derive(Debug, Serialize, Clone)]
struct BrowserCompanionProtocolRequest {
    protocol: &'static str,
    tool_name: String,
    operation: &'static str,
    action_class: &'static str,
    session_scope: String,
    session_id: String,
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct BrowserCompanionProtocolResponse {
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

trait BrowserCompanionRunner {
    fn invoke(
        &self,
        command: &str,
        timeout_seconds: u64,
        request: &BrowserCompanionProtocolRequest,
    ) -> Result<Value, String>;
}

struct CommandBrowserCompanionRunner;

impl BrowserCompanionRunner for CommandBrowserCompanionRunner {
    fn invoke(
        &self,
        command: &str,
        timeout_seconds: u64,
        request: &BrowserCompanionProtocolRequest,
    ) -> Result<Value, String> {
        #[cfg(test)]
        let _test_guard = browser_companion_command_test_lock()
            .lock()
            .map_err(|error| format!("browser companion test lock poisoned: {error}"))?;

        let encoded = serde_json::to_vec(request)
            .map_err(|error| format!("browser_companion_request_encode_failed: {error}"))?;
        let mut child = retry_executable_file_busy(|| {
            let invocation = resolve_command_invocation(command, std::iter::empty::<&str>());
            let mut process = Command::new(&invocation.program);
            process
                .args(&invocation.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            process.spawn()
        })
        .map_err(|error| format!("browser_companion_spawn_failed: {error}"))?;

        let stdin = child.stdin.take();
        write_browser_companion_request(stdin, &encoded, || {
            cleanup_browser_companion_after_stdin_write_failure(&mut child);
        })?;

        let output = wait_for_browser_companion_output(child, timeout_seconds)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            return Err(format!(
                "browser_companion_command_failed: status={} stderr={stderr} stdout={stdout}",
                output.status,
            ));
        }

        let response: BrowserCompanionProtocolResponse = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("browser_companion_protocol_invalid_json: {error}"))?;
        if response.ok {
            return response.result.ok_or_else(|| {
                "browser_companion_protocol_invalid_response: missing result".to_owned()
            });
        }

        Err(format!(
            "browser_companion_protocol_error: {}: {}",
            response.code.unwrap_or_else(|| "unknown_error".to_owned()),
            response
                .message
                .unwrap_or_else(|| "companion reported failure".to_owned())
        ))
    }
}

fn retry_executable_file_busy<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    let pause = || pause_before_browser_companion_spawn_retry(BROWSER_COMPANION_SPAWN_RETRY_DELAY);

    retry_executable_file_busy_with_pause(operation, pause)
}

fn retry_executable_file_busy_with_pause<T, F, P>(operation: F, pause: P) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
    P: FnMut() -> std::io::Result<()>,
{
    retry_spawn_with_pause(operation, BROWSER_COMPANION_SPAWN_RETRY_ATTEMPTS, pause)
}

fn pause_before_browser_companion_spawn_retry(delay: Duration) -> std::io::Result<()> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| {
                handle.block_on(async move {
                    tokio::time::sleep(delay).await;
                })
            });
            Ok(())
        }
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .map_err(|error| {
                            std::io::Error::other(format!(
                                "browser_companion_retry_runtime_create_failed: {error}"
                            ))
                        })?;
                    runtime.block_on(async move {
                        tokio::time::sleep(delay).await;
                    });
                    Ok(())
                })
                .join()
                .map_err(|_panic| {
                    std::io::Error::other("browser_companion_retry_worker_thread_panicked")
                })?
        }),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|error| {
                    std::io::Error::other(format!(
                        "browser_companion_retry_runtime_create_failed: {error}"
                    ))
                })?;
            runtime.block_on(async move {
                tokio::time::sleep(delay).await;
            });
            Ok(())
        }
    }
}

fn write_browser_companion_request<W, C>(
    stdin: Option<W>,
    encoded: &[u8],
    mut cleanup: C,
) -> Result<(), String>
where
    W: Write,
    C: FnMut(),
{
    if let Some(mut stdin) = stdin {
        stdin.write_all(encoded).map_err(|error| {
            cleanup();
            format!("browser_companion_stdin_write_failed: {error}")
        })?;
        stdin.write_all(b"\n").map_err(|error| {
            cleanup();
            format!("browser_companion_stdin_write_failed: {error}")
        })?;
        stdin.flush().map_err(|error| {
            cleanup();
            format!("browser_companion_stdin_write_failed: {error}")
        })?;
    }

    Ok(())
}

fn cleanup_browser_companion_after_stdin_write_failure(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}
fn wait_for_browser_companion_output(
    mut child: std::process::Child,
    timeout_seconds: u64,
) -> Result<std::process::Output, String> {
    let timeout = Duration::from_secs(timeout_seconds.max(1));
    match child.wait_timeout(timeout) {
        Ok(Some(_status)) => child
            .wait_with_output()
            .map_err(|error| format!("browser_companion_wait_failed: {error}")),
        Ok(None) => {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .map_err(|error| format!("browser_companion_wait_failed: {error}"))?;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stderr_suffix = if stderr.is_empty() {
                String::new()
            } else {
                format!(" stderr={stderr}")
            };
            Err(format!(
                "browser_companion_timeout: command exceeded {timeout_seconds}s{stderr_suffix}"
            ))
        }
        Err(error) => Err(format!("browser_companion_wait_failed: {error}")),
    }
}

static NEXT_BROWSER_COMPANION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static BROWSER_COMPANION_SESSIONS: OnceLock<
    Mutex<BTreeMap<String, BTreeMap<String, BrowserCompanionSession>>>,
> = OnceLock::new();

fn browser_companion_sessions()
-> &'static Mutex<BTreeMap<String, BTreeMap<String, BrowserCompanionSession>>> {
    BROWSER_COMPANION_SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
fn browser_companion_command_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn next_browser_companion_sequence() -> u64 {
    NEXT_BROWSER_COMPANION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn execute_browser_companion_core_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let tool_name = request.tool_name.clone();
    let payload = match &request.payload {
        Value::Object(object) => object.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            return Err(format!("{tool_name} payload must be an object"));
        }
    };
    let scope_id = browser_companion_scope_id_from_payload(&payload);
    execute_browser_companion_request(
        request,
        &payload,
        scope_id.as_str(),
        &config.browser_companion,
        &CommandBrowserCompanionRunner,
        true,
    )
}

pub(super) fn execute_browser_companion_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    tool_config: &crate::config::ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_browser_companion_app_tool_with_readiness_override(
        request,
        current_session_id,
        tool_config,
        false,
    )
}

pub(super) fn execute_browser_companion_visible_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    tool_config: &crate::config::ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_browser_companion_app_tool_with_readiness_override(
        request,
        current_session_id,
        tool_config,
        true,
    )
}

fn execute_browser_companion_app_tool_with_readiness_override(
    request: ToolCoreRequest,
    current_session_id: &str,
    tool_config: &crate::config::ToolConfig,
    assume_runtime_ready: bool,
) -> Result<ToolCoreOutcome, String> {
    let tool_name = request.tool_name.clone();
    let payload = match &request.payload {
        Value::Object(object) => object.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            return Err(format!("{tool_name} payload must be an object"));
        }
    };
    let mut policy = if assume_runtime_ready {
        super::runtime_config::browser_companion_runtime_policy_with_env_fallback(tool_config)
    } else {
        super::runtime_config::browser_companion_runtime_policy_from_tool_config(tool_config)
    };
    if assume_runtime_ready {
        policy.ready = true;
    }
    execute_browser_companion_request(
        request,
        &payload,
        current_session_id,
        &policy,
        &CommandBrowserCompanionRunner,
        false,
    )
}

fn execute_browser_companion_request(
    request: ToolCoreRequest,
    payload: &Map<String, Value>,
    scope_id: &str,
    policy: &super::runtime_config::BrowserCompanionRuntimePolicy,
    runner: &dyn BrowserCompanionRunner,
    require_core_operation: bool,
) -> Result<ToolCoreOutcome, String> {
    let operation = BrowserCompanionOperation::from_tool_name(request.tool_name.as_str())
        .ok_or_else(|| {
            format!(
                "tool_not_found: unknown browser companion tool `{}`",
                request.tool_name
            )
        })?;
    if require_core_operation && !operation.is_core() {
        return Err(format!(
            "browser_companion_tool_requires_app_dispatch: {}",
            request.tool_name
        ));
    }
    if !require_core_operation && !operation.is_app() {
        return Err(format!(
            "browser_companion_tool_requires_core_dispatch: {}",
            request.tool_name
        ));
    }

    let command = browser_companion_command(policy)?;
    validate_browser_companion_request_target(
        request.tool_name.as_str(),
        operation,
        payload,
        policy,
    )?;
    let session_id = if operation.requires_existing_session() {
        let session_id =
            required_payload_string(payload, "session_id", request.tool_name.as_str())?;
        ensure_browser_companion_session(scope_id, session_id.as_str())?;
        session_id
    } else {
        format!("browser-companion-{}", next_browser_companion_sequence())
    };

    let protocol_request = BrowserCompanionProtocolRequest {
        protocol: BROWSER_COMPANION_PROTOCOL,
        tool_name: request.tool_name.clone(),
        operation: operation.protocol_name(),
        action_class: operation.action_class(),
        session_scope: scope_id.to_owned(),
        session_id: session_id.clone(),
        arguments: browser_companion_arguments(payload),
    };
    let result = runner.invoke(command, policy.timeout_seconds, &protocol_request)?;
    let result_validation =
        validate_browser_companion_result_target(request.tool_name.as_str(), &result, policy);
    if let Err(error) = result_validation {
        let cleanup = cleanup_browser_companion_after_invalid_result(
            operation,
            scope_id,
            session_id.as_str(),
            command,
            policy.timeout_seconds,
            runner,
        );
        if let Err(cleanup_error) = cleanup {
            return Err(format!("{error}; {cleanup_error}"));
        }
        return Err(error);
    }

    match operation {
        BrowserCompanionOperation::SessionStart => {
            store_browser_companion_session(
                scope_id.to_owned(),
                session_id.clone(),
                BrowserCompanionSession {
                    sequence: next_browser_companion_sequence(),
                },
            )?;
        }
        BrowserCompanionOperation::SessionStop => {
            remove_browser_companion_session(scope_id, session_id.as_str())?;
        }
        BrowserCompanionOperation::Navigate
        | BrowserCompanionOperation::Snapshot
        | BrowserCompanionOperation::Wait
        | BrowserCompanionOperation::Click
        | BrowserCompanionOperation::Type => {
            touch_browser_companion_session(scope_id, session_id.as_str())?;
        }
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "browser-companion",
            "tool_name": request.tool_name,
            "execution_tier": policy.execution_security_tier().as_str(),
            "operation": operation.protocol_name(),
            "action_class": operation.action_class(),
            "session_id": session_id,
            "result": result,
        }),
    })
}

fn cleanup_browser_companion_after_invalid_result(
    operation: BrowserCompanionOperation,
    scope_id: &str,
    session_id: &str,
    command: &str,
    timeout_seconds: u64,
    runner: &dyn BrowserCompanionRunner,
) -> Result<(), String> {
    if operation == BrowserCompanionOperation::SessionStop {
        return Ok(());
    }

    let stop_request = BrowserCompanionProtocolRequest {
        protocol: BROWSER_COMPANION_PROTOCOL,
        tool_name: "browser.companion.session.stop".to_owned(),
        operation: BrowserCompanionOperation::SessionStop.protocol_name(),
        action_class: BrowserCompanionOperation::SessionStop.action_class(),
        session_scope: scope_id.to_owned(),
        session_id: session_id.to_owned(),
        arguments: Value::Object(Map::new()),
    };
    let remote_cleanup = runner.invoke(command, timeout_seconds, &stop_request);
    let local_cleanup = remove_browser_companion_session_if_present(scope_id, session_id);

    let mut cleanup_issues = Vec::new();
    if let Err(error) = remote_cleanup {
        cleanup_issues.push(format!("browser_companion_remote_cleanup_failed: {error}"));
    }
    if let Err(error) = local_cleanup {
        cleanup_issues.push(format!("browser_companion_local_cleanup_failed: {error}"));
    }
    if cleanup_issues.is_empty() {
        return Ok(());
    }

    Err(cleanup_issues.join("; "))
}

fn browser_companion_command(
    policy: &super::runtime_config::BrowserCompanionRuntimePolicy,
) -> Result<&str, String> {
    if !policy.enabled {
        return Err("browser_companion_disabled: tools.browser_companion.enabled=false".to_owned());
    }
    if !policy.ready {
        return Err(
            "browser_companion_not_ready: LOONG_BROWSER_COMPANION_READY is false".to_owned(),
        );
    }
    policy.command.as_deref().ok_or_else(|| {
        "browser_companion_not_configured: tools.browser_companion.command is missing".to_owned()
    })
}

fn browser_companion_scope_id_from_payload(payload: &Map<String, Value>) -> String {
    payload
        .get(super::BROWSER_SESSION_SCOPE_FIELD)
        .or_else(|| payload.get(super::LEGACY_BROWSER_SESSION_SCOPE_FIELD))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BROWSER_COMPANION_SCOPE_ID)
        .to_owned()
}

fn browser_companion_arguments(payload: &Map<String, Value>) -> Value {
    let mut arguments = payload.clone();
    arguments.remove(super::BROWSER_SESSION_SCOPE_FIELD);
    arguments.remove(super::LEGACY_BROWSER_SESSION_SCOPE_FIELD);
    arguments.remove("session_id");
    Value::Object(arguments)
}

fn validate_browser_companion_request_target(
    tool_name: &str,
    operation: BrowserCompanionOperation,
    payload: &Map<String, Value>,
    policy: &super::runtime_config::BrowserCompanionRuntimePolicy,
) -> Result<(), String> {
    let starts_session = operation == BrowserCompanionOperation::SessionStart;
    let navigates = operation == BrowserCompanionOperation::Navigate;
    if !starts_session && !navigates {
        return Ok(());
    }

    let raw_url = required_payload_string(payload, "url", tool_name)?;
    validate_browser_companion_target_url(
        raw_url.as_str(),
        policy,
        format!("{tool_name} payload.url").as_str(),
    )
}

fn validate_browser_companion_result_target(
    tool_name: &str,
    result: &Value,
    policy: &super::runtime_config::BrowserCompanionRuntimePolicy,
) -> Result<(), String> {
    let page_url = result
        .get("page_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(page_url) = page_url else {
        return Ok(());
    };

    validate_browser_companion_target_url(
        page_url,
        policy,
        format!("{tool_name} result.page_url").as_str(),
    )
}

fn validate_browser_companion_target_url(
    raw_url: &str,
    policy: &super::runtime_config::BrowserCompanionRuntimePolicy,
    surface_name: &str,
) -> Result<(), String> {
    let parsed_url = reqwest::Url::parse(raw_url)
        .map_err(|error| format!("{surface_name} is invalid: {error}"))?;
    let web_policy = policy.web_policy();
    let allowed_domains = if web_policy.enforce_allowed_domains {
        Some(&web_policy.allowed_domains)
    } else {
        None
    };
    let blocked_domains = Some(&web_policy.blocked_domains);
    let options = super::web_http::HttpTargetValidationOptions {
        allow_private_hosts: web_policy.allow_private_hosts,
        reject_userinfo: false,
        resolve_dns: false,
        enforce_allowed_domains: web_policy.enforce_allowed_domains,
        allowed_domains,
        blocked_domains,
    };
    super::web_http::validate_http_target(&parsed_url, &options, surface_name).map(|_| ())
}

fn required_payload_string(
    payload: &Map<String, Value>,
    field: &str,
    tool_name: &str,
) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{tool_name} requires payload.{field}"))
}

fn ensure_browser_companion_session(scope_id: &str, session_id: &str) -> Result<(), String> {
    let sessions = browser_companion_sessions()
        .lock()
        .map_err(|error| format!("browser companion session store lock poisoned: {error}"))?;
    if sessions
        .get(scope_id)
        .and_then(|scope_sessions| scope_sessions.get(session_id))
        .is_some()
    {
        return Ok(());
    }
    Err(format!("browser_companion_unknown_session: `{session_id}`"))
}

fn store_browser_companion_session(
    scope_id: String,
    session_id: String,
    session: BrowserCompanionSession,
) -> Result<(), String> {
    let mut sessions = browser_companion_sessions()
        .lock()
        .map_err(|error| format!("browser companion session store lock poisoned: {error}"))?;
    sessions
        .entry(scope_id)
        .or_default()
        .insert(session_id, session);
    Ok(())
}

fn touch_browser_companion_session(scope_id: &str, session_id: &str) -> Result<(), String> {
    let mut sessions = browser_companion_sessions()
        .lock()
        .map_err(|error| format!("browser companion session store lock poisoned: {error}"))?;
    let Some(session) = sessions
        .get_mut(scope_id)
        .and_then(|scope_sessions| scope_sessions.get_mut(session_id))
    else {
        return Err(format!("browser_companion_unknown_session: `{session_id}`"));
    };
    session.sequence = next_browser_companion_sequence();
    Ok(())
}

fn remove_browser_companion_session(scope_id: &str, session_id: &str) -> Result<(), String> {
    let mut sessions = browser_companion_sessions()
        .lock()
        .map_err(|error| format!("browser companion session store lock poisoned: {error}"))?;
    let Some(scope_sessions) = sessions.get_mut(scope_id) else {
        return Err(format!("browser_companion_unknown_session: `{session_id}`"));
    };
    if scope_sessions.remove(session_id).is_none() {
        return Err(format!("browser_companion_unknown_session: `{session_id}`"));
    }
    if scope_sessions.is_empty() {
        sessions.remove(scope_id);
    }
    Ok(())
}

fn remove_browser_companion_session_if_present(
    scope_id: &str,
    session_id: &str,
) -> Result<(), String> {
    let mut sessions = browser_companion_sessions()
        .lock()
        .map_err(|error| format!("browser companion session store lock poisoned: {error}"))?;
    let Some(scope_sessions) = sessions.get_mut(scope_id) else {
        return Ok(());
    };
    scope_sessions.remove(session_id);
    if scope_sessions.is_empty() {
        sessions.remove(scope_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use loong_contracts::ToolCoreRequest;
    use serde_json::{Value, json};

    struct BrokenWriter;

    impl std::io::Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn retry_executable_file_busy_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = super::retry_executable_file_busy(|| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);
            if attempt < 2 {
                Err(std::io::Error::from(std::io::ErrorKind::ExecutableFileBusy))
            } else {
                Ok("spawned")
            }
        })
        .expect("retry should recover once the executable is no longer busy");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn retry_executable_file_busy_surfaces_non_retryable_error_immediately() {
        let attempts = AtomicUsize::new(0);

        let error = super::retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::other("boom"))
        })
        .expect_err("non-retryable spawn errors should surface immediately");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn retry_executable_file_busy_stops_after_retry_budget() {
        let attempts = AtomicUsize::new(0);

        let error = super::retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::from(std::io::ErrorKind::ExecutableFileBusy))
        })
        .expect_err("retry should stop after exhausting the executable-busy budget");

        assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            super::BROWSER_COMPANION_SPAWN_RETRY_ATTEMPTS
        );
    }

    #[test]
    fn retry_executable_file_busy_pauses_between_retryable_failures() {
        let attempts = AtomicUsize::new(0);
        let pauses = AtomicUsize::new(0);

        let result = super::retry_executable_file_busy_with_pause(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);
                if attempt < 2 {
                    Err(std::io::Error::from(std::io::ErrorKind::ExecutableFileBusy))
                } else {
                    Ok("spawned")
                }
            },
            || {
                pauses.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
        .expect("retry should pause between retryable executable-busy failures");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert_eq!(pauses.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn write_browser_companion_request_cleans_up_failed_stdin_writes() {
        let cleaned_up = AtomicBool::new(false);

        let error =
            super::write_browser_companion_request(Some(BrokenWriter), br#"{"ok":true}"#, || {
                cleaned_up.store(true, Ordering::Relaxed);
            })
            .expect_err("stdin write failure should be surfaced");

        assert!(
            error.contains("browser_companion_stdin_write_failed"),
            "expected stdin write failure prefix, got {error}"
        );
        assert!(
            cleaned_up.load(Ordering::Relaxed),
            "stdin write failure should trigger child cleanup"
        );
    }

    #[test]
    fn write_browser_companion_request_appends_newline_frame() {
        let mut buffer = Vec::new();
        let mut writer = io::Cursor::new(&mut buffer);

        super::write_browser_companion_request(Some(&mut writer), br#"{"ok":true}"#, || {})
            .expect("request write should succeed");

        assert_eq!(buffer, b"{\"ok\":true}\n");
    }

    #[test]
    fn pause_before_browser_companion_spawn_retry_is_safe_on_current_thread_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build current-thread runtime");

        runtime.block_on(async {
            super::pause_before_browser_companion_spawn_retry(Duration::from_millis(0))
                .expect("pause should work under a current-thread runtime");
        });
    }

    #[test]
    fn pause_before_browser_companion_spawn_retry_succeeds_without_runtime() {
        super::pause_before_browser_companion_spawn_retry(Duration::ZERO)
            .expect("pause should work without a tokio runtime");
    }

    struct OkRunner;

    impl super::BrowserCompanionRunner for OkRunner {
        fn invoke(
            &self,
            _command: &str,
            _timeout_seconds: u64,
            _request: &super::BrowserCompanionProtocolRequest,
        ) -> Result<Value, String> {
            Ok(json!({
                "navigated": true,
            }))
        }
    }

    struct ResultRunner {
        calls: AtomicUsize,
        requests: Mutex<Vec<super::BrowserCompanionProtocolRequest>>,
        result: Value,
    }

    impl super::BrowserCompanionRunner for ResultRunner {
        fn invoke(
            &self,
            _command: &str,
            _timeout_seconds: u64,
            request: &super::BrowserCompanionProtocolRequest,
        ) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut requests = self
                .requests
                .lock()
                .expect("lock browser companion request log");
            requests.push(request.clone());
            Ok(self.result.clone())
        }
    }

    #[test]
    fn browser_companion_session_start_reports_balanced_execution_tier() {
        let request = ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({"url": "http://127.0.0.1/start"}),
        };
        let payload = request
            .payload
            .as_object()
            .expect("browser companion payload object")
            .clone();
        let policy = super::super::runtime_config::BrowserCompanionRuntimePolicy {
            enabled: true,
            ready: true,
            command: Some("browser-companion".to_owned()),
            expected_version: Some("1.5.0".to_owned()),
            timeout_seconds: 5,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: std::collections::BTreeSet::new(),
            blocked_domains: std::collections::BTreeSet::new(),
        };

        let outcome = super::execute_browser_companion_request(
            request,
            &payload,
            "test-scope",
            &policy,
            &OkRunner,
            true,
        )
        .expect("browser companion session start should succeed");

        assert_eq!(outcome.payload["execution_tier"], json!("balanced"));
        assert_eq!(outcome.payload["action_class"], json!("read"));
        let session_id = outcome.payload["session_id"]
            .as_str()
            .expect("session id in payload")
            .to_owned();

        let stop_request = ToolCoreRequest {
            tool_name: "browser.companion.session.stop".to_owned(),
            payload: json!({"session_id": &session_id}),
        };
        let stop_payload = stop_request
            .payload
            .as_object()
            .expect("browser companion stop payload object")
            .clone();
        let stopped = super::execute_browser_companion_request(
            stop_request,
            &stop_payload,
            "test-scope",
            &policy,
            &OkRunner,
            true,
        )
        .expect("browser companion session stop should succeed");

        assert_eq!(stopped.payload["session_id"], json!(session_id));
        assert_eq!(stopped.payload["operation"], json!("session.stop"));
    }

    #[test]
    fn browser_companion_session_start_rejects_blocked_payload_url_before_spawn() {
        let request = ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({"url": "https://blocked.example.com"}),
        };
        let payload = request
            .payload
            .as_object()
            .expect("browser companion payload object")
            .clone();
        let policy = super::super::runtime_config::BrowserCompanionRuntimePolicy {
            enabled: true,
            ready: true,
            command: Some("browser-companion".to_owned()),
            expected_version: None,
            timeout_seconds: 5,
            allow_private_hosts: false,
            enforce_allowed_domains: false,
            allowed_domains: std::collections::BTreeSet::new(),
            blocked_domains: std::collections::BTreeSet::from(["blocked.example.com".to_owned()]),
        };
        let runner = ResultRunner {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            result: json!({"page_url": "https://blocked.example.com"}),
        };

        let error = super::execute_browser_companion_request(
            request,
            &payload,
            "test-scope",
            &policy,
            &runner,
            true,
        )
        .expect_err("blocked url should fail before companion spawn");

        assert!(
            error.contains("blocked host"),
            "expected blocked-host error, got {error}"
        );
        assert_eq!(runner.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn browser_companion_revalidates_returned_page_url() {
        let request = ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({"url": "http://127.0.0.1/start"}),
        };
        let payload = request
            .payload
            .as_object()
            .expect("browser companion payload object")
            .clone();
        let policy = super::super::runtime_config::BrowserCompanionRuntimePolicy {
            enabled: true,
            ready: true,
            command: Some("browser-companion".to_owned()),
            expected_version: None,
            timeout_seconds: 5,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: std::collections::BTreeSet::new(),
            blocked_domains: std::collections::BTreeSet::from(["internal.example".to_owned()]),
        };
        let runner = ResultRunner {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            result: json!({"page_url": "https://internal.example"}),
        };

        let error = super::execute_browser_companion_request(
            request,
            &payload,
            "test-scope",
            &policy,
            &runner,
            true,
        )
        .expect_err("returned page_url outside policy should fail closed");

        assert!(
            error.contains("blocked host"),
            "expected returned page_url revalidation, got {error}"
        );
        assert_eq!(runner.calls.load(Ordering::Relaxed), 2);

        let requests = runner
            .requests
            .lock()
            .expect("lock browser companion request log");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].operation, "session.start");
        assert_eq!(requests[1].operation, "session.stop");
        assert_eq!(requests[0].session_id, requests[1].session_id);
    }

    #[test]
    fn browser_companion_boundary_failure_drops_existing_session() {
        let scope_id = "test-scope-boundary-cleanup";
        let policy = super::super::runtime_config::BrowserCompanionRuntimePolicy {
            enabled: true,
            ready: true,
            command: Some("browser-companion".to_owned()),
            expected_version: None,
            timeout_seconds: 5,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: std::collections::BTreeSet::new(),
            blocked_domains: std::collections::BTreeSet::from(["internal.example".to_owned()]),
        };
        let start_request = ToolCoreRequest {
            tool_name: "browser.companion.session.start".to_owned(),
            payload: json!({"url": "http://127.0.0.1/start"}),
        };
        let start_payload = start_request
            .payload
            .as_object()
            .expect("browser companion payload object")
            .clone();
        let start_outcome = super::execute_browser_companion_request(
            start_request,
            &start_payload,
            scope_id,
            &policy,
            &OkRunner,
            true,
        )
        .expect("browser companion session start should succeed");
        let session_id = start_outcome.payload["session_id"]
            .as_str()
            .expect("session id should be text")
            .to_owned();

        let navigate_request = ToolCoreRequest {
            tool_name: "browser.companion.navigate".to_owned(),
            payload: json!({
                "session_id": session_id,
                "url": "http://127.0.0.1/next"
            }),
        };
        let navigate_payload = navigate_request
            .payload
            .as_object()
            .expect("browser companion payload object")
            .clone();
        let runner = ResultRunner {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            result: json!({"page_url": "https://internal.example"}),
        };

        let error = super::execute_browser_companion_request(
            navigate_request,
            &navigate_payload,
            scope_id,
            &policy,
            &runner,
            true,
        )
        .expect_err("returned page_url outside policy should fail closed");

        assert!(
            error.contains("blocked host"),
            "expected returned page_url revalidation, got {error}"
        );
        assert!(
            super::ensure_browser_companion_session(scope_id, session_id.as_str()).is_err(),
            "boundary cleanup should drop the existing local companion session"
        );
        assert_eq!(runner.calls.load(Ordering::Relaxed), 2);

        let requests = runner
            .requests
            .lock()
            .expect("lock browser companion request log");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].operation, "navigate");
        assert_eq!(requests[1].operation, "session.stop");
        assert_eq!(requests[0].session_id, requests[1].session_id);
    }
}
