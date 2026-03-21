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
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(&snapshot.config, None);
    let debug_state = snapshot_debug_state(state.as_ref());
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardDebugConsolePayload {
            generated_at: format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            command: "$ loongclaw web debug --readonly".to_owned(),
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
    let personality = prompt_personality_id(snapshot.config.cli.resolved_personality());
    let prompt_mode = if snapshot.config.cli.uses_native_prompt_pack() {
        "native_prompt_pack"
    } else {
        "inline_prompt"
    };
    let memory_profile = snapshot.config.memory.resolved_profile().as_str();
    let enabled_tool_count = build_tool_items(&snapshot.config, runtime)
        .into_iter()
        .filter(|item| item.enabled)
        .count();

    let mut blocks = vec![DashboardDebugConsoleBlockPayload {
        id: "runtime-snapshot".to_owned(),
        kind: "runtime",
        started_at: now.clone(),
        header: format!("{now} runtime snapshot"),
        lines: vec![
            format!(
                "{now} [runtime] ready source=local_daemon provider={active_provider} model={active_model}"
            ),
            format!(
                "{now} [config] prompt={prompt_mode} personality={} memory_profile={memory_profile}",
                personality
            ),
            format!(
                "{now} [provider] endpoint={}",
                snapshot.config.provider.endpoint()
            ),
            format!(
                "{now} [tools] enabled={} approval={} shell_default={}",
                enabled_tool_count,
                approval_mode_label(snapshot.config.tools.approval.mode),
                snapshot.config.tools.shell_default_mode
            ),
        ],
    }];

    blocks.extend(
        debug_state
            .recent_blocks
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|block| DashboardDebugConsoleBlockPayload {
                id: block.id.clone(),
                kind: block.kind,
                started_at: block.started_at.clone(),
                header: block.header.clone(),
                lines: block.lines.clone(),
            }),
    );

    if let Some(log_block) = build_log_output_block() {
        blocks.push(log_block);
    }

    blocks
}

fn snapshot_debug_state(state: &WebApiState) -> DebugConsoleRuntimeState {
    let Ok(debug) = state.debug_state.lock() else {
        return DebugConsoleRuntimeState::default();
    };
    debug.clone()
}

fn build_log_output_block() -> Option<DashboardDebugConsoleBlockPayload> {
    let mut lines = Vec::new();
    append_log_tail(&mut lines, "web-api", default_web_log_root().join("web-api.log"), 10);
    append_log_tail(
        &mut lines,
        "web-api:err",
        default_web_log_root().join("web-api.err.log"),
        8,
    );
    append_log_tail(&mut lines, "web-dev", default_web_log_root().join("web-dev.log"), 8);
    append_log_tail(
        &mut lines,
        "web-dev:err",
        default_web_log_root().join("web-dev.err.log"),
        8,
    );

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

fn default_web_log_root() -> PathBuf {
    mvp::config::default_loongclaw_home().join("logs")
}

fn append_log_tail(lines: &mut Vec<String>, label: &str, path: PathBuf, max_lines: usize) {
    match read_log_tail_lines(path.as_path(), max_lines) {
        Ok(entries) if entries.is_empty() => {}
        Ok(entries) => {
            lines.extend(entries.into_iter().map(|entry| format!("[{label}] {entry}")));
        }
        Err(message) => lines.push(format!("[{label}] unavailable {message}")),
    }
}

fn read_log_tail_lines(path: &std::path::Path, max_lines: usize) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(vec!["(missing)".to_owned()]);
    }

    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let decoded = String::from_utf8_lossy(&bytes);
    let lines = decoded
        .replace('\r', "")
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
        if chars[index] == '\u{1b}' {
            index += 1;
            if index < chars.len() && chars[index] == '[' {
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ('@'..='~').contains(&ch) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }

        output.push(chars[index]);
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
    let mut block = DebugConsoleBlock::operation(
        format!("{kind}:{at}:{}", random::<u32>()),
        kind,
        title,
    );
    block.lines = lines;
    push_debug_block(&mut debug.recent_blocks, block);
}
