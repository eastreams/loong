use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{CliResult, mvp, persist_json_artifact};

pub const TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportSessionSummary {
    pub session_id: String,
    pub kind: String,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub turn_count: usize,
    pub last_turn_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportTurn {
    pub role: String,
    pub content: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportEvent {
    pub id: i64,
    pub session_id: String,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportArtifactDocument {
    pub schema: TrajectoryExportArtifactSchema,
    pub exported_at: String,
    pub session: TrajectoryExportSessionSummary,
    pub turns: Vec<TrajectoryExportTurn>,
    pub events: Vec<TrajectoryExportEvent>,
}

pub fn run_trajectory_export_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    output_path: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, artifact) = collect_trajectory_export_artifact(config_path, session)?;
    let payload = serde_json::to_value(&artifact)
        .map_err(|error| format!("serialize trajectory export artifact failed: {error}"))?;

    if let Some(output_path) = output_path {
        persist_json_artifact(output_path, &payload, "trajectory export artifact")?;
    }

    if as_json {
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize trajectory export output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let resolved_config_path = resolved_path.display().to_string();
    let rendered = format_trajectory_export_text(&resolved_config_path, output_path, &artifact);
    print!("{rendered}");
    Ok(())
}

pub fn collect_trajectory_export_artifact(
    config_path: Option<&str>,
    session: Option<&str>,
) -> CliResult<(PathBuf, TrajectoryExportArtifactDocument)> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)?;
    let session_summary = repo
        .load_session_summary_with_legacy_fallback(&session_id)?
        .ok_or_else(|| format!("session `{session_id}` not found"))?;
    let turn_limit = session_summary.turn_count.max(1);
    let turns = mvp::memory::window_direct(&session_id, turn_limit, &memory_config)
        .map_err(|error| format!("load trajectory transcript failed: {error}"))?;
    let events = collect_trajectory_events(&repo, &session_id)?;
    let exported_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format trajectory export timestamp failed: {error}"))?;

    let artifact = TrajectoryExportArtifactDocument {
        schema: TrajectoryExportArtifactSchema {
            version: TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: "trajectory_export".to_owned(),
            purpose: "session_replay_evidence".to_owned(),
        },
        exported_at,
        session: TrajectoryExportSessionSummary {
            session_id: session_summary.session_id,
            kind: session_summary.kind.as_str().to_owned(),
            parent_session_id: session_summary.parent_session_id,
            label: session_summary.label,
            state: session_summary.state.as_str().to_owned(),
            created_at: session_summary.created_at,
            updated_at: session_summary.updated_at,
            archived_at: session_summary.archived_at,
            turn_count: session_summary.turn_count,
            last_turn_at: session_summary.last_turn_at,
            last_error: session_summary.last_error,
        },
        turns: turns
            .into_iter()
            .map(|turn| TrajectoryExportTurn {
                role: turn.role,
                content: turn.content,
                ts: turn.ts,
            })
            .collect(),
        events: events
            .into_iter()
            .map(|event| TrajectoryExportEvent {
                id: event.id,
                session_id: event.session_id,
                event_kind: event.event_kind,
                actor_session_id: event.actor_session_id,
                payload_json: event.payload_json,
                ts: event.ts,
            })
            .collect(),
    };

    Ok((resolved_path, artifact))
}

fn collect_trajectory_events(
    repo: &mvp::session::repository::SessionRepository,
    session_id: &str,
) -> CliResult<Vec<mvp::session::repository::SessionEventRecord>> {
    let mut after_id = 0i64;
    let mut events = Vec::new();

    loop {
        let page = repo.list_events_after(session_id, after_id, 200)?;
        let page_is_empty = page.is_empty();
        if page_is_empty {
            break;
        }

        let next_after_id = page.last().map(|event| event.id).unwrap_or(after_id);
        after_id = next_after_id;
        events.extend(page);
    }

    Ok(events)
}

pub fn format_trajectory_export_text(
    resolved_config_path: &str,
    output_path: Option<&str>,
    artifact: &TrajectoryExportArtifactDocument,
) -> String {
    let output_label = output_path
        .map(str::to_owned)
        .unwrap_or_else(|| "(stdout)".to_owned());

    [
        format!("schema.version={}", artifact.schema.version),
        format!("config={resolved_config_path}"),
        format!("session_id={}", artifact.session.session_id),
        format!("state={}", artifact.session.state),
        format!("turns={}", artifact.turns.len()),
        format!("events={}", artifact.events.len()),
        format!("output={output_label}"),
    ]
    .join("\n")
        + "\n"
}

pub fn run_trajectory_inspect_cli(artifact_path: &str, as_json: bool) -> CliResult<()> {
    let artifact = load_trajectory_export_artifact(artifact_path)?;

    if as_json {
        let pretty = serde_json::to_string_pretty(&artifact)
            .map_err(|error| format!("serialize trajectory inspect output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = format_trajectory_inspect_text(artifact_path, &artifact);
    print!("{rendered}");
    Ok(())
}

pub fn load_trajectory_export_artifact(
    artifact_path: &str,
) -> CliResult<TrajectoryExportArtifactDocument> {
    let raw_path = PathBuf::from(artifact_path);
    let raw = fs::read_to_string(&raw_path).map_err(|error| {
        format!(
            "read trajectory export artifact {} failed: {error}",
            raw_path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<TrajectoryExportArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode trajectory export artifact {} failed: {error}",
                raw_path.display()
            )
        })?;

    if artifact.schema.version != TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "trajectory export artifact {} uses unsupported schema version {}; expected {}",
            raw_path.display(),
            artifact.schema.version,
            TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }

    if artifact.schema.surface != "trajectory_export" {
        return Err(format!(
            "trajectory export artifact {} uses unsupported schema surface {}",
            raw_path.display(),
            artifact.schema.surface
        ));
    }

    Ok(artifact)
}

pub fn format_trajectory_inspect_text(
    artifact_path: &str,
    artifact: &TrajectoryExportArtifactDocument,
) -> String {
    let first_turn_role = artifact
        .turns
        .first()
        .map(|turn| turn.role.as_str())
        .unwrap_or("-");
    let last_turn_role = artifact
        .turns
        .last()
        .map(|turn| turn.role.as_str())
        .unwrap_or("-");
    let latest_event_kind = artifact
        .events
        .last()
        .map(|event| event.event_kind.as_str())
        .unwrap_or("-");

    [
        format!("schema.version={}", artifact.schema.version),
        format!("artifact={artifact_path}"),
        format!("session_id={}", artifact.session.session_id),
        format!("state={}", artifact.session.state),
        format!("turns={}", artifact.turns.len()),
        format!("events={}", artifact.events.len()),
        format!("first_turn_role={first_turn_role}"),
        format!("last_turn_role={last_turn_role}"),
        format!("latest_event_kind={latest_event_kind}"),
    ]
    .join("\n")
        + "\n"
}
