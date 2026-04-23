use std::path::PathBuf;

use kernel::ToolCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{CliResult, mvp, persist_json_artifact};

const RUNTIME_SESSION_SEARCH_COMMAND: &str = "runtime session search";

pub const SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactResult {
    pub session_id: String,
    pub label: Option<String>,
    pub session_state: String,
    pub archived: bool,
    pub source: String,
    pub source_id: i64,
    pub role: Option<String>,
    pub event_kind: Option<String>,
    pub ts: i64,
    pub snippet: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchArtifactDocument {
    pub schema: SessionSearchArtifactSchema,
    pub exported_at: String,
    pub scope_session_id: String,
    pub query: String,
    pub limit: usize,
    pub include_archived: bool,
    pub include_turns: bool,
    pub include_events: bool,
    pub returned_count: usize,
    pub matched_session_count: usize,
    pub searched_session_count: usize,
    pub results: Vec<SessionSearchArtifactResult>,
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
        return Err(format!(
            "{RUNTIME_SESSION_SEARCH_COMMAND} limit must be >= 1"
        ));
    }

    let query = query.trim();
    let query_is_empty = query.is_empty();
    if query_is_empty {
        return Err(format!(
            "{RUNTIME_SESSION_SEARCH_COMMAND} requires a non-empty --query value"
        ));
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
            "max_results": limit,
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

    let returned_count = payload
        .get("returned_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search tool payload is missing `returned_count`".to_owned())?
        as usize;

    let matched_session_count = payload
        .get("matched_session_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "session-search tool payload is missing `matched_session_count`".to_owned()
        })? as usize;

    let searched_session_count = payload
        .get("searched_session_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "session-search tool payload is missing `searched_session_count`".to_owned()
        })? as usize;

    let include_turns = payload
        .get("include_turns")
        .and_then(Value::as_bool)
        .ok_or_else(|| "session-search tool payload is missing `include_turns`".to_owned())?;

    let include_events = payload
        .get("include_events")
        .and_then(Value::as_bool)
        .ok_or_else(|| "session-search tool payload is missing `include_events`".to_owned())?;

    let results_value = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "session-search tool payload is missing `results`".to_owned())?;

    let results = results_value
        .iter()
        .map(parse_session_search_result)
        .collect::<CliResult<Vec<_>>>()?;
    let result_count = results.len();
    if returned_count != result_count {
        return Err(format!(
            "session-search tool payload returned_count={returned_count} but parsed {result_count} result(s)"
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
        include_turns,
        include_events,
        returned_count,
        matched_session_count,
        searched_session_count,
        results,
    };

    Ok((resolved_path, artifact))
}

fn parse_session_search_result(value: &Value) -> CliResult<SessionSearchArtifactResult> {
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact result is missing `session_id`".to_owned())?
        .to_owned();

    let label = value
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let session_state = value
        .get("session_state")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact result is missing `session_state`".to_owned())?
        .to_owned();

    let archived = value
        .get("archived")
        .and_then(Value::as_bool)
        .ok_or_else(|| "session-search artifact result is missing `archived`".to_owned())?;

    let source = value
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact result is missing `source`".to_owned())?
        .to_owned();
    validate_session_search_source(source.as_str())?;

    let source_id = value
        .get("source_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact result is missing `source_id`".to_owned())?;

    let role = value.get("role").and_then(Value::as_str).map(str::to_owned);

    let event_kind = value
        .get("event_kind")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let ts = value
        .get("ts")
        .and_then(Value::as_i64)
        .ok_or_else(|| "session-search artifact result is missing `ts`".to_owned())?;

    let snippet = value
        .get("snippet")
        .and_then(Value::as_str)
        .ok_or_else(|| "session-search artifact result is missing `snippet`".to_owned())?
        .to_owned();

    let score = value
        .get("score")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session-search artifact result is missing `score`".to_owned())?
        as u32;

    let result = SessionSearchArtifactResult {
        session_id,
        label,
        session_state,
        archived,
        source,
        source_id,
        role,
        event_kind,
        ts,
        snippet,
        score,
    };

    Ok(result)
}

fn validate_session_search_source(source: &str) -> CliResult<()> {
    let supported_source = matches!(source, "turn" | "event");
    if supported_source {
        return Ok(());
    }

    Err(format!("uses unsupported source {source}"))
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
        format!(
            "matched_session_count={} searched_session_count={} include_turns={} include_events={}",
            artifact.matched_session_count,
            artifact.searched_session_count,
            artifact.include_turns,
            artifact.include_events
        ),
    ];

    if artifact.results.is_empty() {
        lines.push("results: -".to_owned());
        return lines.join("\n") + "\n";
    }

    for result in &artifact.results {
        let source_role = result.role.as_deref().unwrap_or("-");
        let source_event_kind = result.event_kind.as_deref().unwrap_or("-");
        let line = format!(
            "- session={} source={} role={} event_kind={} score={} snippet={}",
            result.session_id,
            result.source,
            source_role,
            source_event_kind,
            result.score,
            result.snippet
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

    let result_count = artifact.results.len();
    if artifact.returned_count != result_count {
        return Err(format!(
            "session-search artifact {} says returned_count={} but contains {} result(s)",
            raw_path.display(),
            artifact.returned_count,
            result_count
        ));
    }

    for result in &artifact.results {
        validate_session_search_source(result.source.as_str()).map_err(|error| {
            format!(
                "session-search artifact {} result `{}` {error}",
                raw_path.display(),
                result.source_id
            )
        })?;
    }

    Ok(artifact)
}

pub fn format_session_search_inspect_text(
    artifact_path: &str,
    artifact: &SessionSearchArtifactDocument,
) -> String {
    let first_result = artifact.results.first();
    let first_result_session_id = match first_result {
        Some(result) => result.session_id.as_str(),
        None => "-",
    };
    let first_result_source = match first_result {
        Some(result) => result.source.as_str(),
        None => "-",
    };
    let first_result_role = match first_result {
        Some(result) => result.role.as_deref().unwrap_or("-"),
        None => "-",
    };

    let lines = [
        format!("schema.version={}", artifact.schema.version),
        format!("artifact={artifact_path}"),
        format!("scope_session_id={}", artifact.scope_session_id),
        format!("query={}", artifact.query),
        format!("returned_count={}", artifact.returned_count),
        format!("matched_session_count={}", artifact.matched_session_count),
        format!("searched_session_count={}", artifact.searched_session_count),
        format!("include_turns={}", artifact.include_turns),
        format!("include_events={}", artifact.include_events),
        format!("first_result_session_id={first_result_session_id}"),
        format!("first_result_source={first_result_source}"),
        format!("first_result_role={first_result_role}"),
    ];
    let rendered = lines.join("\n");
    rendered + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_search_errors_use_grouped_runtime_namespace() {
        let zero_limit_error = run_session_search_cli(None, None, "hello", 0, None, false, false)
            .expect_err("zero limit should fail before config loading");
        assert_eq!(
            zero_limit_error,
            "runtime session search limit must be >= 1"
        );

        let empty_query_error = run_session_search_cli(None, None, "   ", 1, None, false, false)
            .expect_err("blank query should fail before config loading");
        assert_eq!(
            empty_query_error,
            "runtime session search requires a non-empty --query value"
        );
    }
}
