use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardDebugConsolePayload {
    generated_at: String,
    command: String,
    blocks: Vec<DashboardDebugConsoleBlockPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardDebugConsoleBlockPayload {
    id: String,
    kind: &'static str,
    started_at: String,
    header: String,
    lines: Vec<String>,
}

pub(super) async fn dashboard_debug_console(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardDebugConsolePayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let tool_runtime =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&snapshot.config, None);
    let debug_state = snapshot_debug_state(state.as_ref());
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardDebugConsolePayload {
            generated_at: format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            command: "$ loong web debug --readonly".to_owned(),
            blocks: build_debug_console_blocks(&snapshot, &tool_runtime, &debug_state),
        },
    }))
}

fn build_debug_console_blocks(
    snapshot: &WebSnapshot,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
    debug_state: &DebugConsoleRuntimeState,
) -> Vec<DashboardDebugConsoleBlockPayload> {
    let now = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
    let active_provider = snapshot.config.active_provider_id().unwrap_or("none");
    let active_model = snapshot.config.provider.model.as_str();
    let enabled_tool_count = build_tool_items(&snapshot.config, runtime)
        .into_iter()
        .filter(|item| item.enabled)
        .count();

    let mut blocks = vec![
        build_turn_summary_block(
            &now,
            snapshot,
            debug_state,
            active_provider,
            active_model,
            enabled_tool_count,
        ),
        build_recent_tool_activity_block(&now, debug_state),
        build_last_failure_block(&now, debug_state),
    ];

    if let Some(log_block) = build_log_output_block() {
        blocks.push(log_block);
    }

    blocks.push(build_raw_events_block(&now, debug_state));

    blocks
}

fn build_turn_summary_block(
    now: &str,
    snapshot: &WebSnapshot,
    debug_state: &DebugConsoleRuntimeState,
    active_provider: &str,
    active_model: &str,
    enabled_tool_count: usize,
) -> DashboardDebugConsoleBlockPayload {
    let latest_turn = debug_state
        .recent_blocks
        .iter()
        .rev()
        .find(|block| block.kind == "turn");

    let status = latest_turn.map(|turn| turn.status).unwrap_or("idle");
    let turn_id = latest_turn
        .map(|turn| turn.id.trim_start_matches("turn:"))
        .unwrap_or("none");
    let session_id = latest_turn
        .and_then(|turn| turn.session_id.as_deref())
        .unwrap_or("none");
    let mut lines = vec![
        format!("[turn] status={status} session={session_id} turn={turn_id}"),
        format!("[provider] ready kind={active_provider} model={active_model}"),
        format!(
            "[tools] active={} recent={} enabled={}",
            debug_state.active_tool_starts.len(),
            debug_state.recent_tool_activity.len(),
            enabled_tool_count
        ),
        format!(
            "[memory] profile={} window={} summary_max_chars={}",
            snapshot.config.memory.resolved_profile().as_str(),
            snapshot.config.memory.sliding_window,
            snapshot.config.memory.summary_max_chars
        ),
    ];

    if let Some(turn) = latest_turn {
        let first_token = turn
            .first_delta_at_ms
            .map(|at| format_duration_ms(at.saturating_sub(turn.started_at_ms)))
            .unwrap_or_else(|| "n/a".to_owned());
        let total = turn
            .finished_at_ms
            .map(|at| format_duration_ms(at.saturating_sub(turn.started_at_ms)))
            .unwrap_or_else(|| "in_progress".to_owned());
        lines.push(format!(
            "[latency] first_token={first_token} total={total} tool_calls={}",
            turn.tool_calls
        ));
    }

    lines.push(format!(
        "[hint] {}",
        latest_turn
            .map(turn_summary_hint)
            .unwrap_or("idle and waiting for the next turn")
    ));

    DashboardDebugConsoleBlockPayload {
        id: "turn-summary".to_owned(),
        kind: "summary",
        started_at: now.to_owned(),
        header: format!("{now} TURN SUMMARY"),
        lines,
    }
}

fn build_recent_tool_activity_block(
    now: &str,
    debug_state: &DebugConsoleRuntimeState,
) -> DashboardDebugConsoleBlockPayload {
    let mut lines = debug_state
        .recent_tool_activity
        .iter()
        .rev()
        .take(5)
        .rev()
        .map(|activity| {
            let duration = activity
                .duration_ms
                .map(|value| format!(" duration={}", format_duration_ms(value)))
                .unwrap_or_default();
            format!(
                "[tool] {} status={}{} detail={}",
                activity.label, activity.outcome, duration, activity.detail
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("[tool] none recent_tool_activity=empty".to_owned());
    }
    DashboardDebugConsoleBlockPayload {
        id: "recent-tool-activity".to_owned(),
        kind: "tools",
        started_at: now.to_owned(),
        header: format!("{now} RECENT TOOL ACTIVITY"),
        lines,
    }
}

fn build_last_failure_block(
    now: &str,
    debug_state: &DebugConsoleRuntimeState,
) -> DashboardDebugConsoleBlockPayload {
    let lines = if let Some(failure) = debug_state.last_failure.as_ref() {
        vec![
            format!("[error] category={} at={}", failure.category, failure.at),
            format!("[detail] {}", failure.detail),
            format!("[hint] {}", failure.hint),
        ]
    } else {
        vec![
            "[error] none recent_failure=none".to_owned(),
            "[hint] no recent failure was recorded".to_owned(),
        ]
    };
    DashboardDebugConsoleBlockPayload {
        id: "last-failure".to_owned(),
        kind: "error",
        started_at: now.to_owned(),
        header: format!("{now} LAST FAILURE"),
        lines,
    }
}

fn build_raw_events_block(
    now: &str,
    debug_state: &DebugConsoleRuntimeState,
) -> DashboardDebugConsoleBlockPayload {
    let mut raw_lines = debug_state
        .recent_blocks
        .iter()
        .flat_map(|block| block.lines.iter().cloned())
        .collect::<Vec<_>>();
    if raw_lines.len() > 18 {
        raw_lines.drain(0..(raw_lines.len() - 18));
    }
    let lines = if raw_lines.is_empty() {
        vec!["[event] none raw_event_buffer=empty".to_owned()]
    } else {
        raw_lines
            .into_iter()
            .map(|line| format!("[event] {line}"))
            .collect()
    };
    DashboardDebugConsoleBlockPayload {
        id: "raw-events".to_owned(),
        kind: "events",
        started_at: now.to_owned(),
        header: format!("{now} RAW EVENTS"),
        lines,
    }
}

fn format_duration_ms(duration_ms: i64) -> String {
    if duration_ms < 1_000 {
        format!("{duration_ms}ms")
    } else {
        format!("{:.2}s", duration_ms as f64 / 1_000.0)
    }
}

fn turn_summary_hint(turn: &DebugConsoleBlock) -> &'static str {
    match turn.status {
        "running" if turn.tool_calls > 0 && turn.first_delta_at_ms.is_none() => {
            "waiting_for_tool_result"
        }
        "running" if turn.first_delta_at_ms.is_some() => "streaming_response",
        "running" => "thinking",
        "completed" => "turn_completed",
        "failed" => "review_last_failure",
        _ => "idle and waiting for the next turn",
    }
}

fn snapshot_debug_state(state: &WebApiState) -> DebugConsoleRuntimeState {
    let Ok(debug) = state.debug_state.lock() else {
        return DebugConsoleRuntimeState::default();
    };
    debug.clone()
}

fn build_log_output_block() -> Option<DashboardDebugConsoleBlockPayload> {
    let mut lines = Vec::new();
    append_log_tail(
        &mut lines,
        "web-api",
        default_web_log_root().join("web-api.log"),
        10,
    );
    append_log_tail(
        &mut lines,
        "web-api:err",
        default_web_log_root().join("web-api.err.log"),
        8,
    );
    append_log_tail(
        &mut lines,
        "web-dev",
        default_web_log_root().join("web-dev.log"),
        8,
    );
    append_log_tail(
        &mut lines,
        "web-dev:err",
        default_web_log_root().join("web-dev.err.log"),
        8,
    );
    let lines = normalize_process_output_lines(lines);

    (!lines.is_empty()).then(|| DashboardDebugConsoleBlockPayload {
        id: "process-output".to_owned(),
        kind: "logs",
        started_at: format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
        header: format!(
            "{} process output",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ),
        lines,
    })
}

fn normalize_process_output_lines(lines: Vec<String>) -> Vec<String> {
    let mut filtered = Vec::with_capacity(lines.len());
    let mut suppressed_optional_repo_note_warnings = 0usize;

    for line in lines {
        if is_optional_repo_note_probe_warning(line.as_str()) {
            suppressed_optional_repo_note_warnings += 1;
            continue;
        }
        filtered.push(line);
    }

    if suppressed_optional_repo_note_warnings > 0 {
        filtered.insert(
            0,
            format!(
                "[web-api:noise] suppressed={} optional repo note lookup warnings",
                suppressed_optional_repo_note_warnings
            ),
        );
    }

    filtered
}

fn is_optional_repo_note_probe_warning(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase();
    normalized.contains("[web-api:err]")
        && normalized.contains("requested_tool_name=file.read")
        && normalized.contains("canonical_tool_name=file.read")
        && normalized.contains("os error 2")
        && ["tools.md", "soul.md", "identity.md", "user.md"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

fn default_web_log_root() -> PathBuf {
    mvp::config::default_loong_home().join("logs")
}

const LOG_TAIL_READ_BYTES: u64 = 128 * 1024;

fn append_log_tail(lines: &mut Vec<String>, label: &str, path: PathBuf, max_lines: usize) {
    match read_log_tail_lines(path.as_path(), max_lines) {
        Ok(entries) if entries.is_empty() => {}
        Ok(entries) => {
            lines.extend(
                entries
                    .into_iter()
                    .map(|entry| format!("[{label}] {entry}")),
            );
        }
        Err(message) => lines.push(format!("[{label}] unavailable {message}")),
    }
}

fn read_log_tail_lines(path: &std::path::Path, max_lines: usize) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(vec!["(missing)".to_owned()]);
    }

    let file_size = fs::metadata(path).map_err(|error| error.to_string())?.len();
    let read_start = file_size.saturating_sub(LOG_TAIL_READ_BYTES);
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(read_start))
        .map_err(|error| error.to_string())?;

    let mut bytes = Vec::with_capacity((file_size - read_start) as usize);
    std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|error| error.to_string())?;

    let normalized = String::from_utf8_lossy(&bytes).replace('\r', "");
    let tail = if read_start > 0 {
        normalized
            .find('\n')
            .map(|index| &normalized[index + 1..])
            .unwrap_or(normalized.as_str())
    } else {
        normalized.as_str()
    };

    let lines = tail
        .lines()
        .rev()
        .take(max_lines)
        .map(strip_ansi_escape_codes)
        .collect::<Vec<_>>();
    Ok(lines.into_iter().rev().collect())
}

fn strip_ansi_escape_codes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if chars.get(index).copied() == Some('\u{1b}') {
            index += 1;
            if chars.get(index).copied() == Some('[') {
                index += 1;
                while index < chars.len() {
                    let ch = chars.get(index).copied().unwrap_or_default();
                    index += 1;
                    if ('@'..='~').contains(&ch) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }

        if let Some(&ch) = chars.get(index) {
            output.push(ch);
        }
        index += 1;
    }

    output
}

pub(super) fn record_debug_operation(
    state: &Arc<WebApiState>,
    kind: &'static str,
    title: String,
    lines: Vec<String>,
) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let at = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
    let mut block =
        DebugConsoleBlock::operation(format!("{kind}:{at}:{}", random::<u32>()), kind, title);
    block.lines = lines;
    push_debug_block(&mut debug.recent_blocks, block);
}
