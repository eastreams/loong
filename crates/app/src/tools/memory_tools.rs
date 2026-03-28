use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Map, Value, json};

use crate::memory::{WorkspaceMemoryDocumentLocation, collect_workspace_memory_document_locations};

const DEFAULT_MEMORY_SEARCH_MAX_RESULTS: usize = 5;
const MAX_MEMORY_SEARCH_RESULTS: usize = 8;
const MEMORY_SEARCH_CONTEXT_RADIUS_LINES: usize = 1;
const DEFAULT_MEMORY_GET_LINES: usize = 40;
const MAX_MEMORY_GET_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemorySearchResult {
    path: String,
    start_line: usize,
    end_line: usize,
    snippet: String,
    score: u32,
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

    let workspace_root = workspace_root_from_config(config)?;
    let locations = collect_workspace_memory_document_locations(workspace_root)?;
    let query_normalized = query.to_ascii_lowercase();
    let query_tokens = tokenize_memory_query(query_normalized.as_str());

    let mut results = Vec::new();
    for location in locations {
        let maybe_result = search_memory_location(
            query_normalized.as_str(),
            query_tokens.as_slice(),
            &location,
        )?;
        let Some(result) = maybe_result else {
            continue;
        };
        results.push(result);
    }

    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.path.cmp(&right.path))
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

    let file_window = read_memory_file_window(
        matched_location.path.as_path(),
        requested_start_line,
        requested_line_count,
    )?;
    let total_lines = file_window.total_lines;
    let selected_lines = file_window.selected_lines;

    if total_lines == 0 {
        return Err(format!("memory file `{}` is empty", matched_location.label));
    }
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

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "path": matched_location.label,
            "start_line": start_line,
            "end_line": end_line,
            "text": text,
        }),
    })
}

pub(super) fn memory_corpus_available(config: &super::runtime_config::ToolRuntimeConfig) -> bool {
    let Some(workspace_root) = config.file_root.as_deref() else {
        return false;
    };

    let result = collect_workspace_memory_document_locations(workspace_root);
    let Ok(locations) = result else {
        return false;
    };

    !locations.is_empty()
}

fn read_memory_file_window(
    path: &Path,
    requested_start_line: usize,
    requested_line_count: usize,
) -> Result<MemoryFileWindow, String> {
    let file = File::open(path)
        .map_err(|error| format!("failed to read memory file {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut total_lines = 0usize;
    let mut selected_lines = Vec::new();
    let mut buffer = String::new();

    loop {
        buffer.clear();

        let bytes_read = reader
            .read_line(&mut buffer)
            .map_err(|error| format!("failed to read memory file {}: {error}", path.display()))?;
        if bytes_read == 0 {
            break;
        }

        total_lines = total_lines.saturating_add(1);

        if total_lines < requested_start_line {
            continue;
        }
        if selected_lines.len() >= requested_line_count {
            break;
        }

        let line = trim_trailing_line_endings(&buffer);
        let owned_line = line.to_owned();

        selected_lines.push(owned_line);

        if selected_lines.len() >= requested_line_count {
            break;
        }
    }

    let file_window = MemoryFileWindow {
        total_lines,
        selected_lines,
    };

    Ok(file_window)
}

fn workspace_root_from_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<&Path, String> {
    config.file_root.as_deref().ok_or_else(|| {
        "memory tools require a configured safe file root before they can access workspace durable memory"
            .to_owned()
    })
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
    location: &WorkspaceMemoryDocumentLocation,
) -> Result<Option<MemorySearchResult>, String> {
    let content = fs::read_to_string(location.path.as_path()).map_err(|error| {
        format!(
            "failed to read memory file {}: {error}",
            location.path.display()
        )
    })?;
    let lines = content.lines().collect::<Vec<_>>();
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
        path: location.label.clone(),
        start_line,
        end_line,
        snippet,
        score: best_match.score,
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

fn memory_search_result_payload(result: &MemorySearchResult) -> Value {
    json!({
        "path": result.path,
        "start_line": result.start_line,
        "end_line": result.end_line,
        "snippet": result.snippet,
        "score": result.score,
    })
}

fn trim_trailing_line_endings(line: &str) -> &str {
    let without_newline = line.strip_suffix('\n').unwrap_or(line);
    let without_carriage_return = without_newline.strip_suffix('\r');
    without_carriage_return.unwrap_or(without_newline)
}
