use std::path::PathBuf;

use kernel::ToolCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{CliResult, mvp, persist_json_artifact};

pub const SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactHitSession {
    pub session_id: String,
    pub kind: String,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
    pub archived_at: Option<i64>,
    pub turn_count: usize,
    pub last_turn_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactHit {
    pub session: SessionSearchArtifactHitSession,
    pub turn_id: i64,
    pub session_turn_index: usize,
    pub role: String,
    pub ts: i64,
    pub snippet: String,
    pub content_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactDocument {
    pub schema: SessionSearchArtifactSchema,
    pub exported_at: String,
    pub scope_session_id: String,
    pub query: String,
    pub limit: usize,
    pub include_archived: bool,
    pub visibility: String,
    pub returned_count: usize,
    pub hits: Vec<SessionSearchArtifactHit>,
}

pub fn run_session_search_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    query: &str,
    limit: usize,
    output_path: Option<&str>,
    include_archived: bool,
    as_json: bool,
) -> CliResult<()> {
    if limit == 0 {
        return Err("session-search limit must be >= 1".to_owned());
    }

    let query = query.trim();
    let query_is_empty = query.is_empty();
    if query_is_empty {
        return Err("session-search requires a non-empty --query value".to_owned());
    }

    let (resolved_path, artifact) =
        collect_session_search_artifact(config_path, session, query, limit, include_archived)?;

    let payload = serde_json::to_value(&artifact)
        .map_err(|error| format!("serialize session-search artifact failed: {error}"))?;

    if let Some(output_path) = output_path {
        persist_json_artifact(output_path, &payload, "session-search artifact")?;
    }

    if as_json {
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize session-search output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let resolved_config_path = resolved_path.display().to_string();
    let rendered = format_session_search_text(&resolved_config_path, output_path, &artifact);
    print!("{rendered}");
    Ok(())
}

pub fn collect_session_search_artifact(
    config_path: Option<&str>,
    session: Option<&str>,
    query: &str,
    limit: usize,
    include_archived: bool,
) -> CliResult<(PathBuf, SessionSearchArtifactDocument)> {
    let (resolved_path, config) = mvp::config::load(config_path)?;

    let scope_session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();

    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);

    let request = ToolCoreRequest {
        tool_name: "session_search".to_owned(),
        payload: serde_json::json!({
            "query": query,
            "limit": limit,
            "include_archived": include_archived,
        }),
    };

    let payload = mvp::tools::execute_app_tool_with_config(
        request,
        &scope_session_id,
        &memory_config,
        &config.tools,
    )?
    .payload;

    let filters = payload
        .get("filters")
        .and_then(Value::as_object)
        .ok_or_else(|| "session-search tool payload is missing `filters`".to_owned())?;
    let visibility = filters
        .get("visibility")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search tool payload is missing `filters.visibility`".to_owned())?;
    validate_session_search_visibility(visibility)?;

    let returned_count = payload
        .get("returned_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search tool payload is missing `returned_count`".to_owned())?
        as usize;

    let hits_value = payload
        .get("hits")
        .and_then(Value::as_array)
        .ok_or_else(|| "session-search tool payload is missing `hits`".to_owned())?;

    let hits = hits_value
        .iter()
        .map(parse_session_search_hit)
        .collect::<CliResult<Vec<_>>>()?;
    let hit_count = hits.len();
    if returned_count != hit_count {
        return Err(format!(
            "session-search tool payload returned_count={returned_count} but parsed {hit_count} hit(s)"
        ));
    }

    let exported_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format session-search artifact timestamp failed: {error}"))?;

    let artifact = SessionSearchArtifactDocument {
        schema: SessionSearchArtifactSchema {
            version: SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: "session_search".to_owned(),
            purpose: "session_recall_evidence".to_owned(),
        },
        exported_at,
        scope_session_id,
        query: query.to_owned(),
        limit,
        include_archived,
        visibility: visibility.to_owned(),
        returned_count,
        hits,
    };

    Ok((resolved_path, artifact))
}

fn parse_session_search_hit(value: &Value) -> CliResult<SessionSearchArtifactHit> {
    let session_value = value
        .get("session")
        .ok_or_else(|| "session-search artifact hit is missing `session`".to_owned())?;

    let session = parse_session_search_hit_session(session_value)?;

    let turn_id = value
        .get("turn_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact hit is missing `turn_id`".to_owned())?;

    let session_turn_index = value
        .get("session_turn_index")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search artifact hit is missing `session_turn_index`".to_owned())?
        as usize;

    let role = value
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact hit is missing `role`".to_owned())?
        .to_owned();

    let ts = value
        .get("ts")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact hit is missing `ts`".to_owned())?;

    let snippet = value
        .get("snippet")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact hit is missing `snippet`".to_owned())?
        .to_owned();

    let content_chars = value
        .get("content_chars")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search artifact hit is missing `content_chars`".to_owned())?
        as usize;

    let hit = SessionSearchArtifactHit {
        session,
        turn_id,
        session_turn_index,
        role,
        ts,
        snippet,
        content_chars,
    };

    Ok(hit)
}

fn parse_session_search_hit_session(value: &Value) -> CliResult<SessionSearchArtifactHitSession> {
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact hit session is missing `session_id`".to_owned())?
        .to_owned();

    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact hit session is missing `kind`".to_owned())?
        .to_owned();

    let parent_session_id = value
        .get("parent_session_id")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let label = value
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let state = value
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact hit session is missing `state`".to_owned())?
        .to_owned();

    let created_at = value
        .get("created_at")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact hit session is missing `created_at`".to_owned())?;

    let updated_at = value
        .get("updated_at")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact hit session is missing `updated_at`".to_owned())?;

    let archived = value
        .get("archived")
        .and_then(Value::as_bool)
        .ok_or_else(|| "session-search artifact hit session is missing `archived`".to_owned())?;

    let archived_at = value.get("archived_at").and_then(Value::as_i64);

    let turn_count = value
        .get("turn_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search artifact hit session is missing `turn_count`".to_owned())?
        as usize;

    let last_turn_at = value.get("last_turn_at").and_then(Value::as_i64);

    let last_error = value
        .get("last_error")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let session = SessionSearchArtifactHitSession {
        session_id,
        kind,
        parent_session_id,
        label,
        state,
        created_at,
        updated_at,
        archived,
        archived_at,
        turn_count,
        last_turn_at,
        last_error,
    };

    Ok(session)
}

pub fn format_session_search_text(
    resolved_config_path: &str,
    output_path: Option<&str>,
    artifact: &SessionSearchArtifactDocument,
) -> String {
    let output_label = output_path
        .map(str::to_owned)
        .unwrap_or_else(|| "(stdout)".to_owned());

    let mut lines = vec![
        format!("config={resolved_config_path}"),
        format!(
            "session_search session={} query={} limit={} include_archived={} returned_count={} output={}",
            artifact.scope_session_id,
            artifact.query,
            artifact.limit,
            artifact.include_archived,
            artifact.returned_count,
            output_label
        ),
    ];

    if artifact.hits.is_empty() {
        lines.push("hits: -".to_owned());
        return lines.join("\n") + "\n";
    }

    for hit in &artifact.hits {
        let line = format!(
            "- session={} turn_index={} role={} snippet={}",
            hit.session.session_id, hit.session_turn_index, hit.role, hit.snippet
        );
        lines.push(line);
    }

    lines.join("\n") + "\n"
}

pub fn run_session_search_inspect_cli(artifact_path: &str, as_json: bool) -> CliResult<()> {
    let artifact = load_session_search_artifact(artifact_path)?;

    if as_json {
        let pretty_result = serde_json::to_string_pretty(&artifact);
        let pretty = pretty_result
            .map_err(|error| format!("serialize session-search inspect output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = format_session_search_inspect_text(artifact_path, &artifact);
    print!("{rendered}");
    Ok(())
}

pub fn load_session_search_artifact(
    artifact_path: &str,
) -> CliResult<SessionSearchArtifactDocument> {
    let raw_path = PathBuf::from(artifact_path);
    let raw = std::fs::read_to_string(&raw_path).map_err(|error| {
        format!(
            "read session-search artifact {} failed: {error}",
            raw_path.display()
        )
    })?;

    let decoded = serde_json::from_str::<SessionSearchArtifactDocument>(&raw);
    let artifact = decoded.map_err(|error| {
        format!(
            "decode session-search artifact {} failed: {error}",
            raw_path.display()
        )
    })?;

    if artifact.schema.version != SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "session-search artifact {} uses unsupported schema version {}; expected {}",
            raw_path.display(),
            artifact.schema.version,
            SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }

    if artifact.schema.surface != "session_search" {
        return Err(format!(
            "session-search artifact {} uses unsupported schema surface {}",
            raw_path.display(),
            artifact.schema.surface
        ));
    }

    validate_session_search_visibility(&artifact.visibility)
        .map_err(|error| format!("session-search artifact {} {error}", raw_path.display()))?;
    let hit_count = artifact.hits.len();
    if artifact.returned_count != hit_count {
        return Err(format!(
            "session-search artifact {} says returned_count={} but contains {} hit(s)",
            raw_path.display(),
            artifact.returned_count,
            hit_count
        ));
    }

    Ok(artifact)
}

fn validate_session_search_visibility(visibility: &str) -> CliResult<()> {
    let supported_visibility = matches!(visibility, "self" | "children");
    if supported_visibility {
        return Ok(());
    }

    Err(format!("uses unsupported visibility {visibility}"))
}

pub fn format_session_search_inspect_text(
    artifact_path: &str,
    artifact: &SessionSearchArtifactDocument,
) -> String {
    let first_hit = artifact.hits.first();
    let first_hit_session_id = match first_hit {
        Some(hit) => hit.session.session_id.as_str(),
        None => "-",
    };
    let first_hit_role = match first_hit {
        Some(hit) => hit.role.as_str(),
        None => "-",
    };

    let lines = [
        format!("schema.version={}", artifact.schema.version),
        format!("artifact={artifact_path}"),
        format!("scope_session_id={}", artifact.scope_session_id),
        format!("query={}", artifact.query),
        format!("returned_count={}", artifact.returned_count),
        format!("visibility={}", artifact.visibility),
        format!("first_hit_session_id={first_hit_session_id}"),
        format!("first_hit_role={first_hit_role}"),
    ];
    let rendered = lines.join("\n");
    rendered + "\n"
}
