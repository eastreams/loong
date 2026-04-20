#[cfg(feature = "tool-shell")]
use loong_contracts::ToolCoreOutcome;
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};
#[cfg(feature = "tool-shell")]
use std::ffi::OsStr;
#[cfg(feature = "tool-shell")]
use std::future::Future;
#[cfg(feature = "tool-shell")]
use std::path::{Path, PathBuf};
#[cfg(feature = "tool-shell")]
use std::process::{ExitStatus, Stdio};
#[cfg(feature = "tool-shell")]
use std::sync::Arc;
#[cfg(feature = "tool-shell")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "tool-shell")]
use std::thread;
#[cfg(feature = "tool-shell")]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(feature = "tool-shell")]
use tokio::fs::File;
#[cfg(feature = "tool-shell")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "tool-shell")]
use tokio::process::Command;

#[cfg(feature = "tool-shell")]
use super::runtime_events::{
    ToolCommandMetrics, ToolOutputDelta, ToolRuntimeEvent, ToolRuntimeEventSink, ToolRuntimeStream,
};
#[cfg(feature = "tool-shell")]
use crate::process_launch::retry_executable_file_busy_async;

#[cfg(feature = "tool-shell")]
pub(super) const DEFAULT_TIMEOUT_MS: u64 = 120_000;
#[cfg(feature = "tool-shell")]
pub(super) const MAX_TIMEOUT_MS: u64 = 600_000;
#[cfg(feature = "tool-shell")]
pub(super) const OUTPUT_CAP_BYTES: usize = 1_048_576;
#[cfg(feature = "tool-shell")]
const SPAWN_RETRY_ATTEMPTS: usize = 5;
#[cfg(feature = "tool-shell")]
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(25);
#[cfg(feature = "tool-shell")]
const FULL_OUTPUT_HANDOFF_DESCRIPTION: &str =
    "Use read with one of the structured recipes below to inspect saved full exec output.";
#[cfg(feature = "tool-shell")]
const PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES: usize = 200;
#[cfg(feature = "tool-shell")]
static NEXT_PROCESS_OUTPUT_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "tool-shell")]
#[derive(Debug)]
pub(super) struct ProcessOutputCapture {
    pub preview: Vec<u8>,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub output_lines: usize,
    pub truncated: bool,
    pub full_output_path: Option<PathBuf>,
    pub full_output_unavailable_reason: Option<String>,
}

#[cfg(feature = "tool-shell")]
impl ProcessOutputCapture {
    pub(super) fn preview_text(&self) -> String {
        String::from_utf8_lossy(&self.preview).trim().to_owned()
    }
}

#[cfg(feature = "tool-shell")]
#[derive(Debug)]
pub(super) struct ProcessExecOutcome {
    pub status: ExitStatus,
    pub stdout: ProcessOutputCapture,
    pub stderr: ProcessOutputCapture,
    pub duration_ms: u64,
}

#[cfg(feature = "tool-shell")]
impl ProcessExecOutcome {
    pub(super) fn is_truncated(&self) -> bool {
        self.stdout.truncated || self.stderr.truncated
    }
}

#[cfg(feature = "tool-shell")]
pub(super) fn resolve_process_cwd_with_config(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
) -> Result<PathBuf, String> {
    let raw_cwd = parse_optional_process_cwd(payload, tool_name)?;

    let resolved_cwd = match raw_cwd {
        Some(raw_cwd) => resolve_process_cwd_override(raw_cwd, config, tool_name)?,
        None => default_process_cwd_with_config(config),
    };

    if !resolved_cwd.is_dir() {
        let display_path = resolved_cwd.display();
        let error = format!("{tool_name} cwd `{display_path}` is not a directory");
        return Err(error);
    }

    Ok(resolved_cwd)
}

#[cfg(feature = "tool-shell")]
fn parse_optional_process_cwd<'a>(
    payload: &'a serde_json::Map<String, Value>,
    tool_name: &str,
) -> Result<Option<&'a str>, String> {
    let Some(raw_value) = payload.get("cwd") else {
        return Ok(None);
    };

    let raw_cwd = raw_value
        .as_str()
        .ok_or_else(|| format!("{tool_name} payload.cwd must be a string"))?;
    let trimmed_cwd = raw_cwd.trim();
    if trimmed_cwd.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed_cwd))
}

#[cfg(feature = "tool-shell")]
fn default_process_cwd_with_config(config: &super::runtime_config::ToolRuntimeConfig) -> PathBuf {
    let configured_root = config.file_root.clone();
    if let Some(configured_root) = configured_root {
        return configured_root;
    }

    let current_dir_result = std::env::current_dir();
    match current_dir_result {
        Ok(current_dir) => current_dir,
        Err(_) => PathBuf::from("."),
    }
}

#[cfg(feature = "tool-shell")]
fn resolve_process_cwd_override(
    raw_cwd: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
) -> Result<PathBuf, String> {
    if config.file_root.is_some() {
        return super::file::resolve_safe_directory_path_with_config(raw_cwd, config);
    }

    let requested_path = PathBuf::from(raw_cwd);
    let base_path = if requested_path.is_absolute() {
        requested_path
    } else {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        current_dir.join(requested_path)
    };

    canonicalize_existing_directory(base_path.as_path(), tool_name)
}

#[cfg(feature = "tool-shell")]
fn canonicalize_existing_directory(path: &Path, tool_name: &str) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to inspect {tool_name} cwd `{display_path}`: {error}")
    })?;

    if !metadata.is_dir() {
        let display_path = path.display();
        let error = format!("{tool_name} cwd `{display_path}` is not a directory");
        return Err(error);
    }

    std::fs::canonicalize(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to canonicalize {tool_name} cwd `{display_path}`: {error}")
    })
}

#[cfg(feature = "tool-shell")]
pub(super) fn run_tool_async<F>(future: F, tool_label: &str) -> Result<F::Output, String>
where
    F: Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            Ok(tokio::task::block_in_place(|| handle.block_on(future)))
        }
        Ok(_) => thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create tokio runtime for {tool_label}: {error}")
                        })?;
                    Ok(rt.block_on(future))
                })
                .join()
                .map_err(|_panic| format!("{tool_label} async worker thread panicked"))?
        }),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    format!("failed to create tokio runtime for {tool_label}: {error}")
                })?;
            Ok(rt.block_on(future))
        }
    }
}

#[cfg(feature = "tool-shell")]
fn stream_label(stream: ToolRuntimeStream) -> &'static str {
    match stream {
        ToolRuntimeStream::Stdout => "stdout",
        ToolRuntimeStream::Stderr => "stderr",
    }
}

#[cfg(feature = "tool-shell")]
fn create_process_output_temp_file(
    stream_name: &str,
    accessible_output_root: Option<&Path>,
) -> Result<(PathBuf, File), String> {
    let output_dir = accessible_output_root
        .map(|root| root.join(".loongclaw/tool-output"))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(output_dir.as_path()).map_err(|error| {
        let display_path = output_dir.display();
        format!("failed to prepare exec output directory `{display_path}`: {error}")
    })?;

    let timestamp_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let process_id = std::process::id();

    for attempt in 0..8_u8 {
        let sequence = NEXT_PROCESS_OUTPUT_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = output_dir.join(format!(
            "loongclaw-process-output-{stream_name}-{process_id}-{timestamp_nanos:x}-{sequence:x}-{attempt}.log"
        ));
        let file_result = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_path());
        let std_file = match file_result {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                let display_path = path.display();
                return Err(format!(
                    "failed to create full {stream_name} output file `{display_path}`: {error}"
                ));
            }
        };
        return Ok((path, File::from_std(std_file)));
    }

    Err(format!(
        "failed to allocate a unique temp file for full {stream_name} output"
    ))
}

#[cfg(feature = "tool-shell")]
async fn read_capped_with_runtime_events<R>(
    mut reader: R,
    cap: usize,
    stream_name: &str,
    stream: ToolRuntimeStream,
    runtime_event_sink: Option<Arc<dyn ToolRuntimeEventSink>>,
    accessible_output_root: Option<PathBuf>,
) -> Result<ProcessOutputCapture, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut preview = Vec::new();
    let mut buffer = [0_u8; 8_192];
    let mut total_bytes = 0_usize;
    let mut newline_count = 0_usize;
    let mut temp_file_path = Option::<PathBuf>::None;
    let mut temp_file = Option::<File>::None;
    let mut full_output_unavailable_reason = Option::<String>::None;
    let mut last_total_byte_was_newline = true;

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("{stream_name} read failed: {error}"))?;
        if read == 0 {
            break;
        }

        let chunk_bytes = buffer.get(..read).unwrap_or(buffer.as_slice());
        total_bytes = total_bytes.saturating_add(read);
        let chunk_newlines = chunk_bytes.iter().filter(|byte| **byte == b'\n').count();
        newline_count = newline_count.saturating_add(chunk_newlines);
        let last_byte_was_newline = chunk_bytes.last().copied() == Some(b'\n');
        last_total_byte_was_newline = last_byte_was_newline;

        let stream_is_truncated = total_bytes > cap;
        if stream_is_truncated && temp_file.is_none() && full_output_unavailable_reason.is_none() {
            match create_process_output_temp_file(stream_name, accessible_output_root.as_deref()) {
                Ok((path, mut file)) => {
                    let backfill_result = if preview.is_empty() {
                        Ok(())
                    } else {
                        file.write_all(preview.as_slice()).await
                    };
                    match backfill_result {
                        Ok(()) => {
                            temp_file_path = Some(path);
                            temp_file = Some(file);
                        }
                        Err(error) => {
                            let display_path = path.display();
                            full_output_unavailable_reason = Some(format!(
                                "failed to backfill full {stream_name} output file `{display_path}`: {error}"
                            ));
                            let _ = tokio::fs::remove_file(path).await;
                        }
                    }
                }
                Err(error) => {
                    full_output_unavailable_reason = Some(error);
                }
            }
        }

        let remaining = cap.saturating_sub(preview.len());
        if remaining > 0 {
            let to_copy = remaining.min(read);
            let retained_chunk = chunk_bytes.get(..to_copy).unwrap_or(chunk_bytes);
            preview.extend(retained_chunk.iter().copied());
        }

        if let Some(file) = temp_file.as_mut() {
            let write_result = file.write_all(chunk_bytes).await;
            if let Err(error) = write_result {
                let display_path = temp_file_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| format!("(unknown {stream_name} temp file)"));
                full_output_unavailable_reason = Some(format!(
                    "failed to persist full {stream_name} output to `{display_path}`: {error}"
                ));
                if let Some(path) = temp_file_path.take() {
                    let _ = tokio::fs::remove_file(path).await;
                }
                temp_file = None;
            }
        }

        let has_partial_final_line = total_bytes > 0 && !last_byte_was_newline;
        let total_lines = newline_count + usize::from(has_partial_final_line);

        if let Some(sink) = runtime_event_sink.as_ref() {
            let chunk_text = String::from_utf8_lossy(chunk_bytes).into_owned();
            let event = ToolRuntimeEvent::OutputDelta(ToolOutputDelta {
                stream,
                chunk: chunk_text,
                total_bytes,
                total_lines,
                truncated: stream_is_truncated,
            });
            sink.emit(event);
        }
    }

    if let Some(mut file) = temp_file {
        let flush_result = file.flush().await;
        if let Err(error) = flush_result {
            let display_path = temp_file_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| format!("(unknown {stream_name} temp file)"));
            full_output_unavailable_reason = Some(format!(
                "failed to flush full {stream_name} output file `{display_path}`: {error}"
            ));
            if let Some(path) = temp_file_path.take() {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
    }

    let last_preview_byte_was_newline = preview.last().copied() == Some(b'\n');
    let preview_newline_count = preview.iter().filter(|byte| **byte == b'\n').count();
    let preview_has_partial_final_line = !preview.is_empty() && !last_preview_byte_was_newline;
    let preview_total_lines = preview_newline_count + usize::from(preview_has_partial_final_line);
    let total_lines = if total_bytes == 0 {
        0
    } else {
        newline_count + usize::from(!last_total_byte_was_newline)
    };

    Ok(ProcessOutputCapture {
        preview,
        total_bytes,
        total_lines,
        output_lines: preview_total_lines,
        truncated: total_bytes > cap,
        full_output_path: temp_file_path,
        full_output_unavailable_reason,
    })
}

#[cfg(feature = "tool-shell")]
fn process_output_capture_details_value(capture: &ProcessOutputCapture) -> Value {
    json!({
        "truncated": capture.truncated,
        "truncated_by": if capture.truncated { Value::String("bytes".to_owned()) } else { Value::Null },
        "total_bytes": capture.total_bytes,
        "total_lines": capture.total_lines,
        "output_bytes": capture.preview.len(),
        "output_lines": capture.output_lines,
        "max_bytes": OUTPUT_CAP_BYTES,
        "full_output_path": capture
            .full_output_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "full_output_unavailable_reason": capture.full_output_unavailable_reason.as_deref(),
    })
}

#[cfg(feature = "tool-shell")]
fn process_output_read_payload(
    path: &Path,
    offset: Option<usize>,
    limit: Option<usize>,
    max_bytes: Option<usize>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("path".to_owned(), Value::String(path.display().to_string()));
    if let Some(offset) = offset {
        payload.insert("offset".to_owned(), json!(offset));
    }
    if let Some(limit) = limit {
        payload.insert("limit".to_owned(), json!(limit));
    }
    if let Some(max_bytes) = max_bytes {
        payload.insert("max_bytes".to_owned(), json!(max_bytes));
    }
    Value::Object(payload)
}

#[cfg(feature = "tool-shell")]
struct ProcessOutputHandoffRecipes {
    recipe_value: Value,
    recommended_recipe: &'static str,
    recommended_reason: &'static str,
    recommended_payload: Value,
}

#[cfg(feature = "tool-shell")]
fn process_output_recommended_recipe(
    capture: &ProcessOutputCapture,
) -> (&'static str, &'static str) {
    if capture.total_lines <= 1 {
        return (
            "wider_bytes",
            "the saved output is effectively one long line, so a wider byte window is the best next read",
        );
    }
    if capture.total_lines > PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES {
        return (
            "last_page",
            "long command output is usually most useful from the tail first",
        );
    }
    (
        "first_page",
        "shorter multi-line output usually fits in the first page",
    )
}

#[cfg(feature = "tool-shell")]
fn process_output_handoff_recipe_value(
    capture: &ProcessOutputCapture,
) -> Option<ProcessOutputHandoffRecipes> {
    let path = capture.full_output_path.as_ref()?;
    let first_page_payload = process_output_read_payload(
        path,
        Some(1),
        Some(PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES),
        None,
    );
    let last_page_offset = capture
        .total_lines
        .saturating_sub(PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES.saturating_sub(1))
        .max(1);
    let last_page_payload = process_output_read_payload(
        path,
        Some(last_page_offset),
        Some(PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES),
        None,
    );
    let wider_bytes_payload =
        process_output_read_payload(path, None, None, Some(8 * OUTPUT_CAP_BYTES));
    let (recommended_recipe, recommended_reason) = process_output_recommended_recipe(capture);
    let recommended_payload = match recommended_recipe {
        "wider_bytes" => wider_bytes_payload.clone(),
        "last_page" => last_page_payload.clone(),
        _ => first_page_payload.clone(),
    };
    let head_payload = first_page_payload.clone();
    let tail_payload = last_page_payload.clone();
    let recipe_value = json!({
        "path": path.display().to_string(),
        "recommended_recipe": recommended_recipe,
        "recommended_reason": recommended_reason,
        "first_page": first_page_payload,
        "last_page": last_page_payload,
        "wider_bytes": wider_bytes_payload,
        // Compatibility aliases while the higher-level prompt/debug surfaces migrate.
        "head": head_payload,
        "tail": tail_payload,
    });

    Some(ProcessOutputHandoffRecipes {
        recipe_value,
        recommended_recipe,
        recommended_reason,
        recommended_payload,
    })
}

#[cfg(feature = "tool-shell")]
fn process_exec_preferred_handoff_stream(
    outcome: &ProcessExecOutcome,
    stdout_recipes: Option<&ProcessOutputHandoffRecipes>,
    stderr_recipes: Option<&ProcessOutputHandoffRecipes>,
) -> Option<&'static str> {
    if !outcome.status.success() && stderr_recipes.is_some() && outcome.stderr.total_bytes > 0 {
        return Some("stderr");
    }
    if stdout_recipes.is_some() && outcome.stdout.total_bytes > 0 {
        return Some("stdout");
    }
    if stderr_recipes.is_some() && outcome.stderr.total_bytes > 0 {
        return Some("stderr");
    }
    if stdout_recipes.is_some() {
        return Some("stdout");
    }
    if stderr_recipes.is_some() {
        return Some("stderr");
    }
    None
}

#[cfg(feature = "tool-shell")]
fn process_output_handoff_value(outcome: &ProcessExecOutcome) -> Option<Value> {
    let stdout_recipes = process_output_handoff_recipe_value(&outcome.stdout);
    let stderr_recipes = process_output_handoff_recipe_value(&outcome.stderr);
    let mut recipes = serde_json::Map::new();

    if let Some(stdout_recipes) = stdout_recipes.as_ref() {
        recipes.insert("stdout".to_owned(), stdout_recipes.recipe_value.clone());
    }
    if let Some(stderr_recipes) = stderr_recipes.as_ref() {
        recipes.insert("stderr".to_owned(), stderr_recipes.recipe_value.clone());
    }

    if recipes.is_empty() {
        return None;
    }

    let preferred_stream = process_exec_preferred_handoff_stream(
        outcome,
        stdout_recipes.as_ref(),
        stderr_recipes.as_ref(),
    );
    let (recommended_recipe, recommended_reason, recommended_payload) = match preferred_stream {
        Some("stderr") => match stderr_recipes.as_ref() {
            Some(recipes) => (
                Some(recipes.recommended_recipe),
                Some(recipes.recommended_reason),
                Some(recipes.recommended_payload.clone()),
            ),
            None => (None, None, None),
        },
        Some("stdout") => match stdout_recipes.as_ref() {
            Some(recipes) => (
                Some(recipes.recommended_recipe),
                Some(recipes.recommended_reason),
                Some(recipes.recommended_payload.clone()),
            ),
            None => (None, None, None),
        },
        _ => (None, None, None),
    };

    Some(json!({
        "tool": "read",
        "description": FULL_OUTPUT_HANDOFF_DESCRIPTION,
        "default_limit": PROCESS_OUTPUT_HANDOFF_PAGE_LIMIT_LINES,
        "supports_offset": true,
        "supports_limit": true,
        "supports_max_bytes": true,
        "recommended_stream": preferred_stream,
        "recommended_recipe": recommended_recipe,
        "recommended_reason": recommended_reason,
        "recommended_payload": recommended_payload,
        "recipes": recipes,
    }))
}

#[cfg(feature = "tool-shell")]
fn process_exec_details_value(outcome: &ProcessExecOutcome) -> Value {
    let mut details = serde_json::Map::new();
    details.insert("duration_ms".to_owned(), json!(outcome.duration_ms));
    details.insert("truncated".to_owned(), Value::Bool(outcome.is_truncated()));
    details.insert(
        "stdout".to_owned(),
        process_output_capture_details_value(&outcome.stdout),
    );
    details.insert(
        "stderr".to_owned(),
        process_output_capture_details_value(&outcome.stderr),
    );

    if let Some(handoff) = process_output_handoff_value(outcome) {
        details.insert("handoff".to_owned(), handoff);
    }

    Value::Object(details)
}

#[cfg(feature = "tool-shell")]
pub(super) fn build_process_tool_outcome(
    tool_name: &str,
    command: &str,
    args: Option<&[String]>,
    cwd: &Path,
    outcome: ProcessExecOutcome,
) -> ToolCoreOutcome {
    let mut payload = serde_json::Map::new();
    payload.insert("adapter".to_owned(), Value::String("core-tools".to_owned()));
    payload.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
    payload.insert("command".to_owned(), Value::String(command.to_owned()));
    if let Some(args) = args {
        payload.insert("args".to_owned(), json!(args));
    }
    payload.insert("cwd".to_owned(), Value::String(cwd.display().to_string()));
    payload.insert("exit_code".to_owned(), json!(outcome.status.code()));
    payload.insert(
        "stdout".to_owned(),
        Value::String(outcome.stdout.preview_text()),
    );
    payload.insert(
        "stderr".to_owned(),
        Value::String(outcome.stderr.preview_text()),
    );
    payload.insert("details".to_owned(), process_exec_details_value(&outcome));

    ToolCoreOutcome {
        status: if outcome.status.success() {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
        payload: Value::Object(payload),
    }
}

#[cfg(feature = "tool-shell")]
pub(super) async fn run_process_with_timeout_with_sink<P, S>(
    program: P,
    args: &[S],
    cwd: &Path,
    timeout_ms: u64,
    error_prefix: &str,
    runtime_event_sink: Option<Arc<dyn ToolRuntimeEventSink>>,
    accessible_output_root: Option<&Path>,
) -> Result<ProcessExecOutcome, String>
where
    P: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    let started_at = Instant::now();
    let mut command = Command::new(program);
    let sanitized_env = loong_contracts::sanitized_child_process_env();

    command.env_clear();
    command.envs(sanitized_env);
    command.args(args);
    command.current_dir(cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());
    command.kill_on_drop(true);

    let mut child = retry_executable_file_busy_async(
        || command.spawn(),
        SPAWN_RETRY_ATTEMPTS,
        SPAWN_RETRY_DELAY,
    )
    .await
    .map_err(|error| format!("{error_prefix} spawn failed: {error}"))?;

    let duration = Duration::from_millis(timeout_ms.max(1));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{error_prefix} stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{error_prefix} stderr pipe missing"))?;

    let metrics_runtime_event_sink = runtime_event_sink.clone();
    let accessible_output_root = accessible_output_root.map(Path::to_path_buf);
    let stdout_runtime_event_sink = runtime_event_sink.clone();
    let stdout_stream_name = stream_label(ToolRuntimeStream::Stdout);
    let stdout_output_root = accessible_output_root.clone();
    let stdout_task = tokio::spawn(async move {
        read_capped_with_runtime_events(
            stdout,
            OUTPUT_CAP_BYTES,
            stdout_stream_name,
            ToolRuntimeStream::Stdout,
            stdout_runtime_event_sink,
            stdout_output_root,
        )
        .await
    });
    let stderr_runtime_event_sink = runtime_event_sink;
    let stderr_stream_name = stream_label(ToolRuntimeStream::Stderr);
    let stderr_output_root = accessible_output_root;
    let stderr_task = tokio::spawn(async move {
        read_capped_with_runtime_events(
            stderr,
            OUTPUT_CAP_BYTES,
            stderr_stream_name,
            ToolRuntimeStream::Stderr,
            stderr_runtime_event_sink,
            stderr_output_root,
        )
        .await
    });

    let result = match tokio::time::timeout(duration, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout_result, stderr_result) = tokio::join!(stdout_task, stderr_task);
            let stdout = stdout_result
                .map_err(|join_error| {
                    format!("{error_prefix} stdout reader panicked: {join_error}")
                })?
                .map_err(|error| format!("{error_prefix} {error}"))?;
            let stderr = stderr_result
                .map_err(|join_error| {
                    format!("{error_prefix} stderr reader panicked: {join_error}")
                })?
                .map_err(|error| format!("{error_prefix} {error}"))?;

            let duration_ms = started_at.elapsed().as_millis();
            let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
            Ok(ProcessExecOutcome {
                status,
                stdout,
                stderr,
                duration_ms,
            })
        }
        Ok(Err(error)) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("{error_prefix} wait failed: {error}"))
        }
        Err(_) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("{error_prefix} timed out after {timeout_ms}ms"))
        }
    };

    if let Some(sink) = metrics_runtime_event_sink.as_ref() {
        let duration_ms = started_at.elapsed().as_millis();
        let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
        let exit_code = result
            .as_ref()
            .ok()
            .and_then(|outcome| outcome.status.code());
        let metrics = ToolCommandMetrics {
            exit_code,
            duration_ms,
        };
        let event = ToolRuntimeEvent::CommandMetrics(metrics);
        sink.emit(event);
    }

    result
}

#[cfg(all(test, feature = "tool-shell"))]
mod tests {
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::process_launch::{
        retry_executable_file_busy_async, should_retry_executable_file_busy,
    };

    #[test]
    fn should_retry_spawn_error_matches_executable_file_busy() {
        let busy_error = Error::from(ErrorKind::ExecutableFileBusy);
        let missing_error = Error::from(ErrorKind::NotFound);

        assert!(should_retry_executable_file_busy(&busy_error));
        assert!(!should_retry_executable_file_busy(&missing_error));
    }

    #[tokio::test]
    async fn retry_executable_file_busy_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy_async(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);

                if attempt < 2 {
                    return Err(Error::from(ErrorKind::ExecutableFileBusy));
                }

                Ok("spawned")
            },
            super::SPAWN_RETRY_ATTEMPTS,
            super::SPAWN_RETRY_DELAY,
        )
        .await
        .expect("executable-file-busy errors should retry");

        let total_attempts = attempts.load(Ordering::Relaxed);

        assert_eq!(result, "spawned");
        assert_eq!(total_attempts, 3);
    }
}
