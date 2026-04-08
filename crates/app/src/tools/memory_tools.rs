use std::fs;
use std::path::Path;

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Map, Value, json};

use crate::memory::{
    MemoryContextProvenance, MemoryProvenanceSourceKind, MemoryRecallMode,
    ParsedWorkspaceMemoryDocument, WorkspaceMemoryDocumentLocation,
    collect_workspace_memory_document_locations, parse_workspace_memory_document,
};

const DEFAULT_MEMORY_SEARCH_MAX_RESULTS: usize = 5;
const MAX_MEMORY_SEARCH_RESULTS: usize = 8;
const MEMORY_SEARCH_CONTEXT_RADIUS_LINES: usize = 1;
const DEFAULT_MEMORY_GET_LINES: usize = 40;
const MAX_MEMORY_GET_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemorySearchResult {
    source: &'static str,
    path: Option<String>,
    session_id: Option<String>,
    scope: Option<String>,
    kind: Option<String>,
    role: Option<String>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    snippet: String,
    score: u32,
    provenance: MemoryContextProvenance,
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BestLineMatch {
    line_number: usize,
    score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryFileWindow {
    total_lines: usize,
    selected_lines: Vec<String>,
}

pub(super) fn execute_memory_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let tool_name = request.tool_name.as_str();
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory_search payload must be an object".to_owned())?;

    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory_search requires payload.query".to_owned())?;

    let max_results = parse_optional_usize_field(
        payload,
        tool_name,
        "max_results",
        DEFAULT_MEMORY_SEARCH_MAX_RESULTS,
        1,
        Some(MAX_MEMORY_SEARCH_RESULTS),
    )?;

    let query_normalized = query.to_ascii_lowercase();
    let query_tokens = tokenize_memory_query(query_normalized.as_str());

    let mut results = Vec::new();
    if let Some(workspace_root) = config.file_root.as_deref() {
        let locations = collect_workspace_memory_document_locations(workspace_root)?;
        for location in locations {
            let maybe_result = search_memory_location(
                query_normalized.as_str(),
                query_tokens.as_slice(),
                config,
                &location,
            )?;
            let Some(result) = maybe_result else {
                continue;
            };
            results.push(result);
        }
    }
    results.extend(search_canonical_memory_results(
        query_normalized.as_str(),
        query_tokens.as_slice(),
        max_results,
        config,
    )?);

    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.source.cmp(right.source))
            .then(left.path.cmp(&right.path))
            .then(left.session_id.cmp(&right.session_id))
            .then(left.start_line.cmp(&right.start_line))
    });

    let bounded_results = results.into_iter().take(max_results).collect::<Vec<_>>();
    let result_payload = bounded_results
        .iter()
        .map(memory_search_result_payload)
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "query": query,
            "returned": result_payload.len(),
            "results": result_payload,
        }),
    })
}

pub(super) fn execute_memory_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let tool_name = request.tool_name.as_str();
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory_get payload must be an object".to_owned())?;

    let raw_path = payload
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory_get requires payload.path".to_owned())?;

    let requested_start_line = parse_optional_usize_field(payload, tool_name, "from", 1, 1, None)?;
    let requested_line_count = parse_optional_usize_field(
        payload,
        tool_name,
        "lines",
        DEFAULT_MEMORY_GET_LINES,
        1,
        Some(MAX_MEMORY_GET_LINES),
    )?;

    let workspace_root = workspace_root_from_config(config)?;
    let locations = collect_workspace_memory_document_locations(workspace_root)?;
    let resolved_path = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
    let matched_location = find_memory_location_for_path(&locations, resolved_path.as_path())?
        .ok_or_else(|| {
            format!(
                "memory_get path `{raw_path}` is not part of the workspace durable memory corpus"
            )
        })?;

    let raw_content = read_workspace_memory_text_lossy(matched_location.path.as_path())?;
    let maybe_parsed_document = parse_workspace_memory_document(
        raw_content.as_str(),
        matched_location,
        config.selected_memory_system_id.as_str(),
        MemoryRecallMode::OperatorInspection,
    )?;
    let Some(parsed_document) = maybe_parsed_document else {
        return Err(format!("memory file `{}` is empty", matched_location.label));
    };

    let record_status = parsed_document
        .provenance
        .record_status
        .unwrap_or(crate::memory::MemoryRecordStatus::Active);
    if !record_status.is_active() {
        return Err(format!(
            "memory file `{}` is inactive and cannot be inspected",
            matched_location.label
        ));
    }

    let file_window = read_memory_file_window(
        parsed_document.body.as_str(),
        requested_start_line,
        requested_line_count,
    );
    let total_lines = file_window.total_lines;
    let selected_lines = file_window.selected_lines;
    if requested_start_line > total_lines {
        return Err(format!(
            "memory_get start line {} exceeds file length {} for `{}`",
            requested_start_line, total_lines, matched_location.label
        ));
    }

    let start_line = requested_start_line;
    let selected_line_count = selected_lines.len();
    let end_line = start_line
        .saturating_add(selected_line_count)
        .saturating_sub(1);
    let text = selected_lines.join("\n");
    let provenance = &parsed_document.provenance;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "path": matched_location.label,
            "start_line": start_line,
            "end_line": end_line,
            "text": text,
            "provenance": provenance,
            "metadata": memory_document_metadata_payload(&parsed_document),
        }),
    })
}

pub(super) fn memory_corpus_available(config: &super::runtime_config::ToolRuntimeConfig) -> bool {
    let workspace_memory_available = workspace_memory_corpus_available(config);

    if workspace_memory_available {
        return true;
    }

    config
        .memory_sqlite_path
        .as_deref()
        .is_some_and(|path| path.exists())
}

pub(super) fn workspace_memory_corpus_available(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> bool {
    config
        .file_root
        .as_deref()
        .and_then(|workspace_root| collect_workspace_memory_document_locations(workspace_root).ok())
        .is_some_and(|locations| !locations.is_empty())
}

fn workspace_root_from_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<&Path, String> {
    config.file_root.as_deref().ok_or_else(|| {
        "memory tools require a configured safe file root before they can access workspace durable memory"
            .to_owned()
    })
}

fn read_memory_file_window(
    content: &str,
    requested_start_line: usize,
    requested_line_count: usize,
) -> MemoryFileWindow {
    let mut total_lines = 0usize;
    let mut selected_lines = Vec::new();
    for line in content.lines() {
        total_lines = total_lines.saturating_add(1);

        if total_lines < requested_start_line {
            continue;
        }
        if selected_lines.len() >= requested_line_count {
            break;
        }

        selected_lines.push(line.to_owned());
    }

    MemoryFileWindow {
        total_lines,
        selected_lines,
    }
}

fn parse_optional_usize_field(
    payload: &Map<String, Value>,
    tool_name: &str,
    field_name: &str,
    default_value: usize,
    min_value: usize,
    max_value: Option<usize>,
) -> Result<usize, String> {
    let Some(raw_value) = payload.get(field_name) else {
        return Ok(default_value);
    };

    let maybe_integer = raw_value.as_u64();
    let Some(integer_value) = maybe_integer else {
        return Err(invalid_numeric_field_message(
            tool_name, field_name, min_value, max_value,
        ));
    };
    let parsed_value = usize::try_from(integer_value).map_err(|conversion_error| {
        format!(
            "{}: {conversion_error}",
            invalid_numeric_field_message(tool_name, field_name, min_value, max_value)
        )
    })?;
    if parsed_value < min_value {
        return Err(invalid_numeric_field_message(
            tool_name, field_name, min_value, max_value,
        ));
    }

    if let Some(max_value) = max_value
        && parsed_value > max_value
    {
        return Err(invalid_numeric_field_message(
            tool_name,
            field_name,
            min_value,
            Some(max_value),
        ));
    }

    Ok(parsed_value)
}

fn invalid_numeric_field_message(
    tool_name: &str,
    field_name: &str,
    min_value: usize,
    max_value: Option<usize>,
) -> String {
    match max_value {
        Some(max_value) => {
            format!(
                "{tool_name} payload.{field_name} must be an integer between {min_value} and {max_value}"
            )
        }
        None => {
            format!(
                "{tool_name} payload.{field_name} must be an integer greater than or equal to {min_value}"
            )
        }
    }
}

fn search_memory_location(
    query: &str,
    query_tokens: &[String],
    config: &super::runtime_config::ToolRuntimeConfig,
    location: &WorkspaceMemoryDocumentLocation,
) -> Result<Option<MemorySearchResult>, String> {
    let content = read_workspace_memory_text_lossy(location.path.as_path())?;
    let maybe_parsed_document = parse_workspace_memory_document(
        content.as_str(),
        location,
        config.selected_memory_system_id.as_str(),
        MemoryRecallMode::OperatorInspection,
    )?;
    let Some(parsed_document) = maybe_parsed_document else {
        return Ok(None);
    };

    let record_status = parsed_document
        .provenance
        .record_status
        .unwrap_or(crate::memory::MemoryRecordStatus::Active);
    if !record_status.is_active() {
        return Ok(None);
    }

    let lines = parsed_document.body.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Ok(None);
    }

    let maybe_match = best_line_match(query, query_tokens, lines.as_slice());
    let Some(best_match) = maybe_match else {
        return Ok(None);
    };

    let total_lines = lines.len();
    let context_radius = MEMORY_SEARCH_CONTEXT_RADIUS_LINES;
    let (start_line, end_line) =
        snippet_window(total_lines, best_match.line_number, context_radius);
    let start_index = start_line.saturating_sub(1);
    let end_index = end_line;
    let snippet_lines = lines
        .get(start_index..end_index)
        .ok_or_else(|| "memory_search selected snippet window is out of bounds".to_owned())?;
    let snippet = snippet_lines.join("\n");

    let result = MemorySearchResult {
        source: "workspace_file",
        path: Some(location.label.clone()),
        session_id: None,
        scope: None,
        kind: None,
        role: None,
        start_line: Some(start_line),
        end_line: Some(end_line),
        snippet,
        score: best_match.score,
        provenance: parsed_document.provenance.clone(),
        metadata: Some(memory_document_metadata_payload(&parsed_document)),
    };

    Ok(Some(result))
}

fn best_line_match(query: &str, query_tokens: &[String], lines: &[&str]) -> Option<BestLineMatch> {
    let mut best_match: Option<BestLineMatch> = None;

    for (index, line) in lines.iter().enumerate() {
        let score = line_match_score(query, query_tokens, line);
        if score == 0 {
            continue;
        }

        let line_number = index.saturating_add(1);
        let candidate = BestLineMatch { line_number, score };
        let should_replace = match best_match {
            None => true,
            Some(current) => {
                candidate.score > current.score
                    || (candidate.score == current.score
                        && candidate.line_number < current.line_number)
            }
        };
        if should_replace {
            best_match = Some(candidate);
        }
    }

    best_match
}

fn line_match_score(query: &str, query_tokens: &[String], line: &str) -> u32 {
    let normalized_line = line.to_ascii_lowercase();
    let mut score = 0u32;

    if normalized_line.contains(query) {
        score = score.saturating_add(100);
    }

    let mut matched_token_count = 0u32;
    for token in query_tokens {
        if normalized_line.contains(token) {
            matched_token_count = matched_token_count.saturating_add(1);
            score = score.saturating_add(20);
        }
    }

    if matched_token_count > 1 && matched_token_count as usize == query_tokens.len() {
        score = score.saturating_add(20);
    }

    score
}

fn tokenize_memory_query(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn snippet_window(total_lines: usize, focus_line: usize, context_radius: usize) -> (usize, usize) {
    let start_line = focus_line.saturating_sub(context_radius).max(1);
    let end_line = focus_line
        .saturating_add(context_radius)
        .min(total_lines)
        .max(start_line);
    (start_line, end_line)
}

fn find_memory_location_for_path<'a>(
    locations: &'a [WorkspaceMemoryDocumentLocation],
    resolved_path: &Path,
) -> Result<Option<&'a WorkspaceMemoryDocumentLocation>, String> {
    let resolved_key = normalized_requested_path_key(resolved_path);

    for location in locations {
        let location_key = normalized_existing_path_key(location.path.as_path())?;
        if location_key == resolved_key {
            return Ok(Some(location));
        }
    }

    Ok(None)
}

fn normalized_requested_path_key(path: &Path) -> String {
    let normalized_path = super::normalize_without_fs(path);
    normalized_path.display().to_string()
}

fn normalized_existing_path_key(path: &Path) -> Result<String, String> {
    let canonical_path = path.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize workspace memory path {}: {error}",
            path.display()
        )
    })?;
    Ok(canonical_path.display().to_string())
}

fn search_canonical_memory_results(
    query: &str,
    query_tokens: &[String],
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<MemorySearchResult>, String> {
    let Some(sqlite_path) = config.memory_sqlite_path.as_ref() else {
        return Ok(Vec::new());
    };
    if !sqlite_path.exists() {
        return Ok(Vec::new());
    }

    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(sqlite_path.clone()),
        ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
    };
    let hits = crate::memory::search_canonical_memory(query, max_results, None, &memory_config)?;

    Ok(hits
        .into_iter()
        .map(|hit| {
            let score = canonical_memory_match_score(query, query_tokens, &hit);
            let scope = hit.record.scope.as_str().to_owned();
            let kind = hit.record.kind.as_str().to_owned();
            let provenance = canonical_memory_hit_provenance(config, &hit);
            MemorySearchResult {
                source: "canonical_session",
                path: None,
                session_id: Some(hit.record.session_id),
                scope: Some(scope),
                kind: Some(kind),
                role: hit.record.role,
                start_line: None,
                end_line: None,
                snippet: hit.record.content,
                score,
                provenance,
                metadata: None,
            }
        })
        .collect())
}

fn canonical_memory_match_score(
    query: &str,
    query_tokens: &[String],
    hit: &crate::memory::CanonicalMemorySearchHit,
) -> u32 {
    let content_score = line_match_score(query, query_tokens, hit.record.content.as_str());
    let session_id_score = line_match_score(query, query_tokens, hit.record.session_id.as_str());
    let scope_score = line_match_score(query, query_tokens, hit.record.scope.as_str());
    let kind_score = line_match_score(query, query_tokens, hit.record.kind.as_str());
    let role_score = hit
        .record
        .role
        .as_deref()
        .map(|value| line_match_score(query, query_tokens, value))
        .unwrap_or(0);
    let metadata_text = hit.record.metadata.to_string();
    let metadata_score = line_match_score(query, query_tokens, metadata_text.as_str());

    let strongest_metadata_score = session_id_score
        .max(scope_score)
        .max(kind_score)
        .max(role_score)
        .max(metadata_score);
    let strongest_score = content_score.max(strongest_metadata_score);

    strongest_score.max(1)
}

fn memory_search_result_payload(result: &MemorySearchResult) -> Value {
    json!({
        "source": result.source,
        "path": result.path,
        "session_id": result.session_id,
        "scope": result.scope,
        "kind": result.kind,
        "role": result.role,
        "start_line": result.start_line,
        "end_line": result.end_line,
        "snippet": result.snippet,
        "score": result.score,
        "provenance": result.provenance,
        "metadata": result.metadata,
    })
}

fn canonical_memory_hit_provenance(
    config: &super::runtime_config::ToolRuntimeConfig,
    hit: &crate::memory::CanonicalMemorySearchHit,
) -> MemoryContextProvenance {
    let source_label = Some(hit.record.kind.as_str().to_owned());
    let source_path = None;
    let scope = Some(hit.record.scope);

    MemoryContextProvenance::new(
        config.selected_memory_system_id.as_str(),
        MemoryProvenanceSourceKind::CanonicalMemoryRecord,
        source_label,
        source_path,
        scope,
        MemoryRecallMode::OperatorInspection,
    )
    .with_trust_level(crate::memory::MemoryTrustLevel::Session)
    .with_authority(crate::memory::MemoryAuthority::Advisory)
    .with_record_status(crate::memory::MemoryRecordStatus::Active)
}

fn read_workspace_memory_text_lossy(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read memory file {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(bytes.as_slice()).into_owned();

    Ok(text)
}

fn memory_document_metadata_payload(document: &ParsedWorkspaceMemoryDocument) -> Value {
    let provenance = &document.provenance;
    let derived_kind = provenance.derived_kind.map(|kind| kind.as_str().to_owned());
    let freshness_ts = provenance.freshness_ts;
    let record_status = provenance
        .record_status
        .map(|status| status.as_str().to_owned());
    let trust_level = provenance
        .trust_level
        .map(|trust| trust.as_str().to_owned());
    let authority = provenance
        .authority
        .map(|authority| authority.as_str().to_owned());
    let superseded_by = provenance.superseded_by.clone();

    json!({
        "derived_kind": derived_kind,
        "freshness_ts": freshness_ts,
        "record_status": record_status,
        "trust_level": trust_level,
        "authority": authority,
        "superseded_by": superseded_by,
        "body_line_offset": document.body_line_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_returns_cross_session_canonical_hits_without_workspace_root() {
        let root = unique_temp_dir("loongclaw-memory-search-canonical");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");

        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        crate::memory::append_turn_direct(
            "release-session",
            "assistant",
            "Deployment cutoff is 17:00 Beijing time and requires a release note.",
            &memory_config,
        )
        .expect("append canonical assistant turn");

        let runtime_config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: None,
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "deployment cutoff release note",
                    "max_results": 4
                }),
            },
            &runtime_config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["source"], "canonical_session");
        assert_eq!(results[0]["session_id"], "release-session");
        assert_eq!(results[0]["scope"], "session");
        assert_eq!(results[0]["kind"], "assistant_turn");
        assert_eq!(results[0]["role"], "assistant");
        assert!(
            results[0]["path"].is_null(),
            "canonical search should not synthesize a file path: {results:?}"
        );
        assert!(
            results[0]["start_line"].is_null() && results[0]["end_line"].is_null(),
            "canonical search should not report line windows: {results:?}"
        );
        assert!(
            results[0]["snippet"]
                .as_str()
                .is_some_and(|value| value.contains("17:00 Beijing time")),
            "expected canonical snippet in result payload: {results:?}"
        );
        assert_eq!(results[0]["provenance"]["memory_system_id"], "builtin");
        assert_eq!(
            results[0]["provenance"]["source_kind"],
            "canonical_memory_record"
        );
        assert_eq!(results[0]["provenance"]["scope"], "session");
        assert_eq!(
            results[0]["provenance"]["recall_mode"],
            "operator_inspection"
        );
    }

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_preserves_metadata_only_canonical_hits() {
        let root = unique_temp_dir("loongclaw-memory-search-canonical-metadata");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");

        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loongclaw_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();
        crate::memory::append_turn_direct(
            "metadata-session",
            "assistant",
            &payload,
            &memory_config,
        )
        .expect("append structured canonical turn");

        let runtime_config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: None,
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "workspace-import",
                    "max_results": 4
                }),
            },
            &runtime_config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["source"], "canonical_session");
        assert_eq!(results[0]["session_id"], "metadata-session");
        assert_eq!(results[0]["scope"], "workspace");
        assert_eq!(results[0]["kind"], "imported_profile");
        assert_eq!(
            results[0]["provenance"]["source_kind"],
            "canonical_memory_record"
        );
        assert_eq!(results[0]["provenance"]["scope"], "workspace");
        assert!(
            results[0]["score"].as_u64().is_some_and(|value| value > 0),
            "metadata-only matches should keep a stable score: {results:?}"
        );
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn memory_search_tool_skips_tombstoned_workspace_memory_files() {
        let root = unique_temp_dir("loongclaw-memory-search-tombstoned");
        let memory_dir = root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(
            root.join("MEMORY.md"),
            concat!(
                "---\n",
                "status: tombstoned\n",
                "---\n",
                "Deploy freeze window is Friday.\n",
            ),
        )
        .expect("write tombstoned memory");
        std::fs::write(
            memory_dir.join("2026-03-23.md"),
            "Deploy freeze window remains Friday for customer migration.\n",
        )
        .expect("write daily log");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "deploy freeze window",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert!(
            results.iter().any(|entry| entry["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("2026-03-23.md"))),
            "expected a matching active document to remain searchable: {results:?}"
        );
        assert!(
            results.iter().all(|entry| entry["path"] != "MEMORY.md"),
            "tombstoned documents should be filtered from search results: {results:?}"
        );
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn memory_get_tool_strips_frontmatter_and_surfaces_workspace_metadata() {
        let root = unique_temp_dir("loongclaw-memory-get-frontmatter");
        let memory_path = root.join("MEMORY.md");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(
            &memory_path,
            concat!(
                "---\n",
                "kind: procedure\n",
                "trust: workspace_curated\n",
                "status: active\n",
                "---\n",
                "line one\n",
                "line two\n",
            ),
        )
        .expect("write root memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_get".to_owned(),
                payload: json!({
                    "path": "MEMORY.md",
                    "from": 1,
                    "lines": 2
                }),
            },
            &config,
        )
        .expect("memory get should succeed");

        assert_eq!(outcome.payload["text"], "line one\nline two");
        assert_eq!(outcome.payload["metadata"]["derived_kind"], "procedure");
        assert_eq!(
            outcome.payload["metadata"]["trust_level"],
            "workspace_curated"
        );
        assert_eq!(outcome.payload["metadata"]["body_line_offset"], 5);
    }
}
