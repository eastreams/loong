use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use clap::{Args, Subcommand};
use loongclaw_spec::CliResult;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub use loongclaw_app::session::trajectory::RuntimeTrajectoryApprovalRequest;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryArtifactDocument;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryArtifactSchema;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryCanonicalRecord;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryExportMode;
pub use loongclaw_app::session::trajectory::RuntimeTrajectorySession;
pub use loongclaw_app::session::trajectory::RuntimeTrajectorySessionEvent;
pub use loongclaw_app::session::trajectory::RuntimeTrajectorySessionSummary;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryStatistics;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryTerminalOutcome;
pub use loongclaw_app::session::trajectory::RuntimeTrajectoryTurnRecord;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTrajectoryCommands {
    /// Export one persisted session trajectory artifact from local runtime state
    Export(RuntimeTrajectoryExportCommandOptions),
    /// Load and render one persisted runtime trajectory artifact
    Show(RuntimeTrajectoryShowCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryExportCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long, default_value_t = false)]
    pub lineage: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryShowCommandOptions {
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub fn run_runtime_trajectory_cli(command: RuntimeTrajectoryCommands) -> CliResult<()> {
    match command {
        RuntimeTrajectoryCommands::Export(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_trajectory_export_command(options)?;
            emit_runtime_trajectory_artifact(&artifact, as_json)
        }
        RuntimeTrajectoryCommands::Show(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_trajectory_show_command(options)?;
            emit_runtime_trajectory_artifact(&artifact, as_json)
        }
    }
}

pub fn execute_runtime_trajectory_export_command(
    options: RuntimeTrajectoryExportCommandOptions,
) -> CliResult<RuntimeTrajectoryArtifactDocument> {
    let session_id = normalized_required_session_id(options.session.as_str())?;
    let export_mode = export_mode_from_flag(options.lineage);
    let (_, config) = crate::mvp::config::load(options.config.as_deref())?;
    let memory_config =
        crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let exported_at = now_rfc3339()?;
    let artifact = crate::mvp::session::trajectory::export_runtime_trajectory(
        session_id.as_str(),
        export_mode,
        &memory_config,
        exported_at.as_str(),
    )?;

    if let Some(output) = options.output.as_deref() {
        persist_runtime_trajectory_artifact(output, &artifact)?;
    }

    Ok(artifact)
}

pub fn execute_runtime_trajectory_show_command(
    options: RuntimeTrajectoryShowCommandOptions,
) -> CliResult<RuntimeTrajectoryArtifactDocument> {
    let artifact_path = Path::new(&options.artifact);
    load_runtime_trajectory_artifact(artifact_path)
}

fn emit_runtime_trajectory_artifact(
    artifact: &RuntimeTrajectoryArtifactDocument,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(artifact)
            .map_err(|error| format!("serialize runtime trajectory artifact failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_runtime_trajectory_text(artifact);
    println!("{rendered}");
    Ok(())
}

fn normalized_required_session_id(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("runtime-trajectory export requires --session".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn export_mode_from_flag(lineage: bool) -> RuntimeTrajectoryExportMode {
    if lineage {
        return RuntimeTrajectoryExportMode::Lineage;
    }

    RuntimeTrajectoryExportMode::SessionOnly
}

fn now_rfc3339() -> CliResult<String> {
    let timestamp = OffsetDateTime::now_utc();
    let formatted = timestamp
        .format(&Rfc3339)
        .map_err(|error| format!("format trajectory export timestamp failed: {error}"))?;
    Ok(formatted)
}

fn persist_runtime_trajectory_artifact(
    output: &str,
    artifact: &RuntimeTrajectoryArtifactDocument,
) -> CliResult<()> {
    let output_path = Path::new(output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime trajectory artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    let pretty = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize runtime trajectory artifact failed: {error}"))?;
    fs::write(output_path, pretty).map_err(|error| {
        format!(
            "write runtime trajectory artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

fn load_runtime_trajectory_artifact(path: &Path) -> CliResult<RuntimeTrajectoryArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime trajectory artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<RuntimeTrajectoryArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode runtime trajectory artifact {} failed: {error}",
                path.display()
            )
        })?;
    Ok(artifact)
}

pub fn render_runtime_trajectory_text(artifact: &RuntimeTrajectoryArtifactDocument) -> String {
    let mut lines = Vec::new();
    let stats = &artifact.statistics;
    let kind_rollup = format_equals_rollup(&stats.canonical_kind_counts);
    let conversation_event_rollup = format_equals_rollup(&stats.conversation_event_name_counts);
    let tool_intent_rollup = format_equals_rollup(&stats.tool_intent_status_counts);

    let header = format!(
        "runtime trajectory export requested_session_id={} root_session_id={} export_mode={} exported_at={}",
        artifact.requested_session_id,
        artifact.root_session_id,
        artifact.export_mode.as_str(),
        artifact.exported_at,
    );
    lines.push(header);

    let summary = format!(
        "sessions={} turns={} terminal_outcomes={} session_events={} approval_requests={}",
        stats.session_count,
        stats.turn_count,
        stats.terminal_outcome_count,
        stats.session_event_count,
        stats.approval_request_count,
    );
    lines.push(summary);

    let kind_counts = format!("canonical_kind_counts={kind_rollup}");
    lines.push(kind_counts);
    let conversation_event_counts =
        format!("conversation_event_name_counts={conversation_event_rollup}");
    lines.push(conversation_event_counts);
    let tool_intent_counts = format!("tool_intent_status_counts={tool_intent_rollup}");
    lines.push(tool_intent_counts);

    for session in &artifact.sessions {
        let terminal_state = terminal_state_label(session.terminal_outcome.is_some());
        let session_line = format!(
            "- session_id={} kind={} state={} depth={} turns={} events={} approvals={} terminal_outcome={}",
            session.summary.session_id,
            session.summary.kind,
            session.summary.state,
            session.lineage_depth,
            session.turns.len(),
            session.session_events.len(),
            session.approval_requests.len(),
            terminal_state,
        );
        lines.push(session_line);
    }

    lines.join("\n")
}

fn terminal_state_label(has_terminal_outcome: bool) -> &'static str {
    if has_terminal_outcome {
        "present"
    } else {
        "absent"
    }
}

fn format_equals_rollup(entries: &BTreeMap<String, usize>) -> String {
    if entries.is_empty() {
        return "-".to_owned();
    }

    let mut parts = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let part = format!("{key}={value}");
        parts.push(part);
    }
    parts.join(",")
}
