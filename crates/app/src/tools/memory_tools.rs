use std::fs;
use std::path::Path;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Map, Value, json};

use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::memory::{
    BuiltinMemorySystem, MemoryContextEntry, MemoryContextKind, MemoryContextProvenance,
    MemoryProvenanceSourceKind, MemoryRecallMode, MemoryRetrievalIntent, MemoryRetrievalOutcome,
    MemoryRetrievalRequest, MemoryRetrievalResult, MemoryRetrievalStrategy, MemoryScope,
    MemorySystem, MemoryTrustLevel, ParsedWorkspaceMemoryDocument, WorkspaceMemoryDocumentKind,
    WorkspaceMemoryDocumentLocation, collect_workspace_memory_document_locations,
    memory_injection_reason_for_intent, memory_retrieval_provenance_summary,
    memory_retrieval_reason_for_request, parse_workspace_memory_document,
};
use crate::search_text::{normalize_search_text, tokenize_normalized_search_text};

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

fn parse_memory_retrieve_intent(
    payload: &Map<String, Value>,
) -> Result<MemoryRetrievalIntent, String> {
    let Some(raw_intent) = payload.get("intent") else {
        return Ok(MemoryRetrievalIntent::OperatorInspection);
    };
    let raw_intent = raw_intent
        .as_str()
        .ok_or_else(|| "memory.retrieve payload.intent must be a string".to_owned())?;
    match raw_intent.trim().to_ascii_lowercase().as_str() {
        "" | "operator_inspection" => Ok(MemoryRetrievalIntent::OperatorInspection),
        "prompt_assembly" => Ok(MemoryRetrievalIntent::PromptAssembly),
        other => Err(format!(
            "memory.retrieve payload.intent `{other}` must be one of `operator_inspection` or `prompt_assembly`"
        )),
    }
}

fn build_memory_retrieve_bundle(
    session_id: &str,
    query: Option<String>,
    intent: MemoryRetrievalIntent,
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<MemoryRetrievalOutcome, String> {
    let results = if let Some(query) = query.as_deref() {
        search_memory_retrieval_results(session_id, query, max_results, config)?
    } else {
        retrieve_prompt_memory_results(session_id, max_results, config)?
    };

    let retrieval_request = MemoryRetrievalRequest {
        session_id: session_id.to_owned(),
        memory_system_id: config.selected_memory_system_id.clone(),
        strategy: if query.is_some() {
            MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
        } else {
            MemoryRetrievalStrategy::WorkspaceReferenceOnly
        },
        planning_notes: Vec::new(),
        query: query.clone(),
        recall_mode: match intent {
            MemoryRetrievalIntent::OperatorInspection => MemoryRecallMode::OperatorInspection,
            MemoryRetrievalIntent::PromptAssembly => MemoryRecallMode::PromptAssembly,
        },
        scopes: vec![MemoryScope::Workspace, MemoryScope::Session],
        budget_items: max_results.max(1),
        allowed_kinds: Vec::new(),
    };

    Ok(MemoryRetrievalOutcome {
        query,
        intent,
        prompt_eligible: !results.is_empty(),
        retrieval_reason: memory_retrieval_reason_for_request(&retrieval_request).to_owned(),
        injection_reason: memory_injection_reason_for_intent(intent).to_owned(),
        results,
    })
}

fn build_memory_runtime_config_for_tools(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> MemoryRuntimeConfig {
    MemoryRuntimeConfig {
        sqlite_path: config.memory_sqlite_path.clone(),
        resolved_system_id: Some(config.selected_memory_system_id.clone()),
        summary_max_chars: config.browser.max_text_chars.max(256),
        ..MemoryRuntimeConfig::default()
    }
}

fn search_memory_retrieval_results(
    session_id: &str,
    query: &str,
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<MemoryRetrievalResult>, String> {
    let query_normalized = normalize_search_text(query);
    let query_tokens = tokenize_memory_query(query_normalized.as_str());

    let mut results: Vec<MemoryRetrievalResult> = Vec::new();
    if let Some(workspace_root) = config.file_root.as_deref() {
        if let Some(indexed_results) = search_indexed_workspace_memory_results(
            query_normalized.as_str(),
            query_tokens.as_slice(),
            max_results,
            config,
            workspace_root,
        )? {
            results.extend(
                indexed_results
                    .into_iter()
                    .map(memory_search_result_to_retrieval_result),
            );
        } else {
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
                results.push(memory_search_result_to_retrieval_result(result));
            }
        }
    }
    results.extend(
        search_canonical_memory_results_for_session(
            session_id,
            query_normalized.as_str(),
            query_tokens.as_slice(),
            max_results,
            config,
        )?
        .into_iter()
        .map(memory_search_result_to_retrieval_result),
    );

    sort_memory_retrieval_results(&mut results);
    Ok(results.into_iter().take(max_results).collect())
}

fn retrieve_prompt_memory_results(
    session_id: &str,
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<MemoryRetrievalResult>, String> {
    let memory_runtime_config = build_memory_runtime_config_for_tools(config);
    let system = BuiltinMemorySystem;
    let request = MemoryRetrievalRequest {
        session_id: session_id.to_owned(),
        memory_system_id: memory_runtime_config.selected_system_id().to_owned(),
        strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
        planning_notes: vec!["memory.retrieve prompt retrieval".to_owned()],
        query: None,
        recall_mode: MemoryRecallMode::PromptAssembly,
        scopes: vec![MemoryScope::Workspace, MemoryScope::Session],
        budget_items: max_results.max(1),
        allowed_kinds: Vec::new(),
    };
    let entries = system
        .run_retrieve_stage(
            &request,
            config.file_root.as_deref(),
            &memory_runtime_config,
            &[],
        )?
        .unwrap_or_default();
    let ranked_entries = system
        .run_rank_stage(entries, &memory_runtime_config)?
        .unwrap_or_default();

    let results = ranked_entries
        .into_iter()
        .filter(|entry| entry.kind == MemoryContextKind::RetrievedMemory)
        .map(memory_entry_to_retrieval_result)
        .collect::<Vec<_>>();
    Ok(results.into_iter().take(max_results).collect())
}

fn memory_entry_to_retrieval_result(entry: MemoryContextEntry) -> MemoryRetrievalResult {
    let provenance = entry.provenance.first().cloned().unwrap_or_else(|| {
        MemoryContextProvenance::new(
            "builtin",
            MemoryProvenanceSourceKind::WorkspaceDocument,
            None,
            None,
            Some(MemoryScope::Workspace),
            MemoryRecallMode::PromptAssembly,
        )
        .with_trust_level(MemoryTrustLevel::Derived)
    });
    let snippet = entry.content.trim().to_owned();
    MemoryRetrievalResult {
        source: "retrieved_memory".to_owned(),
        path: provenance.source_label.clone(),
        session_id: None,
        scope: provenance.scope.map(|scope| scope.as_str().to_owned()),
        kind: Some(memory_context_kind_id(entry.kind).to_owned()),
        role: Some(entry.role),
        start_line: None,
        end_line: None,
        snippet,
        score: 1,
        provenance: provenance.clone(),
        provenance_summary: memory_retrieval_provenance_summary(&provenance),
        metadata: None,
    }
}

fn memory_context_kind_id(kind: MemoryContextKind) -> &'static str {
    match kind {
        MemoryContextKind::Profile => "profile",
        MemoryContextKind::Summary => "summary",
        MemoryContextKind::Derived => "derived",
        MemoryContextKind::RetrievedMemory => "retrieved_memory",
        MemoryContextKind::Turn => "turn",
    }
}

fn memory_retrieve_result_payload(result: &MemoryRetrievalResult) -> Value {
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
        "provenance_summary": result.provenance_summary,
        "metadata": result.metadata,
    })
}

fn sort_memory_retrieval_results(results: &mut [MemoryRetrievalResult]) {
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.source.cmp(&right.source))
            .then(left.path.cmp(&right.path))
            .then(left.session_id.cmp(&right.session_id))
            .then(left.start_line.cmp(&right.start_line))
    });
}

fn memory_search_result_to_retrieval_result(result: MemorySearchResult) -> MemoryRetrievalResult {
    let provenance_summary = memory_retrieval_provenance_summary(&result.provenance);
    MemoryRetrievalResult {
        source: result.source.to_owned(),
        path: result.path,
        session_id: result.session_id,
        scope: result.scope,
        kind: result.kind,
        role: result.role,
        start_line: result.start_line,
        end_line: result.end_line,
        snippet: result.snippet,
        score: result.score,
        provenance: result.provenance,
        provenance_summary,
        metadata: result.metadata,
    }
}

pub(super) fn execute_memory_retrieve_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.retrieve payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.retrieve requires payload.session_id".to_owned())?;
    let intent = parse_memory_retrieve_intent(payload)?;
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let max_results = parse_optional_usize_field(
        payload,
        "memory.retrieve",
        "max_results",
        DEFAULT_MEMORY_SEARCH_MAX_RESULTS,
        1,
        Some(MAX_MEMORY_SEARCH_RESULTS),
    )?;

    let bundle = build_memory_retrieve_bundle(session_id, query, intent, max_results, config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "session_id": session_id,
            "query": bundle.query,
            "intent": bundle.intent.as_str(),
            "prompt_eligible": bundle.prompt_eligible,
            "retrieval_reason": bundle.retrieval_reason,
            "injection_reason": bundle.injection_reason,
            "returned": bundle.results.len(),
            "results": bundle.results.iter().map(memory_retrieve_result_payload).collect::<Vec<_>>(),
        }),
    })
}

pub(super) fn execute_memory_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
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
        request.tool_name.as_str(),
        "max_results",
        DEFAULT_MEMORY_SEARCH_MAX_RESULTS,
        1,
        Some(MAX_MEMORY_SEARCH_RESULTS),
    )?;

    let bundle = build_memory_retrieve_bundle(
        "__operator_memory_search__",
        Some(query.to_owned()),
        MemoryRetrievalIntent::OperatorInspection,
        max_results,
        config,
    )?;
    let result_payload = bundle
        .results
        .iter()
        .map(|result| {
            let search_result = MemorySearchResult {
                source: Box::leak(result.source.clone().into_boxed_str()),
                path: result.path.clone(),
                session_id: result.session_id.clone(),
                scope: result.scope.clone(),
                kind: result.kind.clone(),
                role: result.role.clone(),
                start_line: result.start_line,
                end_line: result.end_line,
                snippet: result.snippet.clone(),
                score: result.score,
                provenance: result.provenance.clone(),
                metadata: result.metadata.clone(),
            };
            memory_search_result_payload(&search_result)
        })
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "query": query,
            "intent": bundle.intent.as_str(),
            "prompt_eligible": bundle.prompt_eligible,
            "retrieval_reason": bundle.retrieval_reason,
            "injection_reason": bundle.injection_reason,
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

    let scope = parsed_document
        .provenance
        .scope
        .map(|scope| scope.as_str().to_owned());
    let kind = parsed_document
        .provenance
        .derived_kind
        .map(|kind| kind.as_str().to_owned());
    let metadata = memory_document_metadata_payload(&parsed_document);
    let metadata_score = workspace_memory_match_score(
        query,
        query_tokens,
        location.label.as_str(),
        scope.as_deref(),
        kind.as_deref(),
        &metadata,
    );
    let best_body_match = best_line_match(query, query_tokens, lines.as_slice());
    let focus_line = best_body_match
        .map(|matched| matched.line_number)
        .or_else(|| (metadata_score > 0).then_some(1));
    let Some(focus_line) = focus_line else {
        return Ok(None);
    };

    let total_lines = lines.len();
    let context_radius = MEMORY_SEARCH_CONTEXT_RADIUS_LINES;
    let (start_line, end_line) = snippet_window(total_lines, focus_line, context_radius);
    let start_index = start_line.saturating_sub(1);
    let end_index = end_line;
    let snippet_lines = lines
        .get(start_index..end_index)
        .ok_or_else(|| "memory_search selected snippet window is out of bounds".to_owned())?;
    let snippet = snippet_lines.join("\n");
    let score = best_body_match
        .map(|matched| matched.score)
        .unwrap_or(0)
        .max(metadata_score)
        .max(1);

    let result = MemorySearchResult {
        source: "workspace_file",
        path: Some(location.label.clone()),
        session_id: None,
        scope,
        kind,
        role: None,
        start_line: Some(start_line),
        end_line: Some(end_line),
        snippet,
        score,
        provenance: parsed_document.provenance.clone(),
        metadata: Some(metadata),
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
    let normalized_line = normalize_search_text(line);
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
    tokenize_normalized_search_text(query)
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
    let canonical_path = dunce::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize workspace memory path {}: {error}",
            path.display()
        )
    })?;
    let normalized_path = dunce::simplified(&canonical_path);
    Ok(normalized_path.display().to_string())
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

fn workspace_memory_match_score(
    query: &str,
    query_tokens: &[String],
    label: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    metadata: &Value,
) -> u32 {
    let label_score = line_match_score(query, query_tokens, label);
    let scope_score = scope
        .map(|scope| line_match_score(query, query_tokens, scope))
        .unwrap_or(0);
    let kind_score = kind
        .map(|kind| line_match_score(query, query_tokens, kind))
        .unwrap_or(0);
    let metadata_text = metadata.to_string();
    let metadata_score = line_match_score(query, query_tokens, metadata_text.as_str());

    label_score
        .max(scope_score)
        .max(kind_score)
        .max(metadata_score)
}

fn search_indexed_workspace_memory_results(
    query: &str,
    query_tokens: &[String],
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
    workspace_root: &Path,
) -> Result<Option<Vec<MemorySearchResult>>, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (query, query_tokens, max_results, config, workspace_root);
        Ok(None)
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let Some(sqlite_path) = config.memory_sqlite_path.as_ref() else {
            return Ok(None);
        };
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(sqlite_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        let hits = crate::memory::search_workspace_memory_documents(
            query,
            max_results,
            workspace_root,
            config.selected_memory_system_id.as_str(),
            &memory_config,
        )?;

        let mut results = Vec::new();
        for hit in hits {
            let maybe_result =
                indexed_workspace_memory_hit_to_result(query, query_tokens, config, &hit)?;
            let Some(result) = maybe_result else {
                continue;
            };
            results.push(result);
        }

        Ok(Some(results))
    }
}

fn indexed_workspace_memory_hit_to_result(
    query: &str,
    query_tokens: &[String],
    config: &super::runtime_config::ToolRuntimeConfig,
    hit: &crate::memory::WorkspaceMemoryIndexedSearchHit,
) -> Result<Option<MemorySearchResult>, String> {
    let lines = hit.body.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Ok(None);
    }

    let scope = workspace_memory_scope(hit.document_kind)
        .as_str()
        .to_owned();
    let kind = hit.derived_kind.as_str().to_owned();
    let metadata = indexed_workspace_memory_metadata_payload(hit);
    let metadata_score = workspace_memory_match_score(
        query,
        query_tokens,
        hit.label.as_str(),
        Some(scope.as_str()),
        Some(kind.as_str()),
        &metadata,
    );
    let best_body_match = best_line_match(query, query_tokens, lines.as_slice());
    let focus_line = best_body_match
        .map(|matched| matched.line_number)
        .or_else(|| (metadata_score > 0).then_some(1));
    let Some(focus_line) = focus_line else {
        return Ok(None);
    };
    let total_lines = lines.len();
    let (start_line, end_line) =
        snippet_window(total_lines, focus_line, MEMORY_SEARCH_CONTEXT_RADIUS_LINES);
    let snippet_lines = lines
        .get(start_line.saturating_sub(1)..end_line)
        .ok_or_else(|| {
            "memory_search selected indexed workspace snippet window is out of bounds".to_owned()
        })?;
    let snippet = snippet_lines.join("\n");
    let score = best_body_match
        .map(|matched| matched.score)
        .unwrap_or(0)
        .max(metadata_score)
        .max(1);

    Ok(Some(MemorySearchResult {
        source: "workspace_file",
        path: Some(hit.label.clone()),
        session_id: None,
        scope: Some(scope),
        kind: Some(kind),
        role: None,
        start_line: Some(start_line),
        end_line: Some(end_line),
        snippet,
        score,
        provenance: indexed_workspace_memory_hit_provenance(config, hit),
        metadata: Some(metadata),
    }))
}

fn workspace_memory_scope(
    document_kind: WorkspaceMemoryDocumentKind,
) -> crate::memory::MemoryScope {
    match document_kind {
        WorkspaceMemoryDocumentKind::Curated => crate::memory::MemoryScope::Workspace,
        WorkspaceMemoryDocumentKind::DailyLog => crate::memory::MemoryScope::Session,
    }
}

fn indexed_workspace_memory_hit_provenance(
    config: &super::runtime_config::ToolRuntimeConfig,
    hit: &crate::memory::WorkspaceMemoryIndexedSearchHit,
) -> MemoryContextProvenance {
    let scope = Some(workspace_memory_scope(hit.document_kind));

    let mut provenance = MemoryContextProvenance::new(
        config.selected_memory_system_id.as_str(),
        MemoryProvenanceSourceKind::WorkspaceDocument,
        Some(hit.label.clone()),
        Some(hit.path.clone()),
        scope,
        MemoryRecallMode::OperatorInspection,
    )
    .with_trust_level(hit.trust_level)
    .with_authority(hit.authority)
    .with_derived_kind(hit.derived_kind)
    .with_content_hash(hit.content_hash.clone())
    .with_record_status(hit.record_status);

    if let Some(freshness_ts) = hit.freshness_ts {
        provenance = provenance.with_freshness_ts(freshness_ts);
    }
    if let Some(ref superseded_by) = hit.superseded_by {
        provenance = provenance.with_superseded_by(superseded_by.clone());
    }

    provenance
}

fn indexed_workspace_memory_metadata_payload(
    hit: &crate::memory::WorkspaceMemoryIndexedSearchHit,
) -> Value {
    json!({
        "derived_kind": hit.derived_kind.as_str(),
        "freshness_ts": hit.freshness_ts,
        "record_status": hit.record_status.as_str(),
        "trust_level": hit.trust_level.as_str(),
        "authority": hit.authority.as_str(),
        "superseded_by": hit.superseded_by,
        "body_line_offset": hit.body_line_offset,
    })
}

fn search_canonical_memory_results_for_session(
    session_id: &str,
    query: &str,
    query_tokens: &[String],
    max_results: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<MemorySearchResult>, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, query, query_tokens, max_results, config);
        Ok(Vec::new())
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config = build_memory_runtime_config_for_tools(config);
        let hits = crate::memory::search_canonical_memory(
            query,
            max_results,
            Some(session_id),
            &memory_config,
        )?;
        let mut results = Vec::new();
        for hit in hits {
            let maybe_result = canonical_memory_hit_to_result(query, query_tokens, config, &hit)?;
            let Some(result) = maybe_result else {
                continue;
            };
            results.push(result);
        }
        Ok(results)
    }
}

fn canonical_memory_hit_to_result(
    query: &str,
    query_tokens: &[String],
    config: &super::runtime_config::ToolRuntimeConfig,
    hit: &crate::memory::CanonicalMemorySearchHit,
) -> Result<Option<MemorySearchResult>, String> {
    let lines = hit.record.content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Ok(None);
    }

    let metadata = json!({
        "session_id": hit.record.session_id,
        "scope": hit.record.scope.as_str(),
        "kind": hit.record.kind.as_str(),
        "role": hit.record.role,
    });
    let metadata_score = canonical_memory_match_score(query, query_tokens, hit);
    let best_body_match = best_line_match(query, query_tokens, lines.as_slice());
    let focus_line = best_body_match
        .map(|matched| matched.line_number)
        .or_else(|| (metadata_score > 0).then_some(1));
    let Some(focus_line) = focus_line else {
        return Ok(None);
    };
    let total_lines = lines.len();
    let (start_line, end_line) =
        snippet_window(total_lines, focus_line, MEMORY_SEARCH_CONTEXT_RADIUS_LINES);
    let snippet_lines = lines
        .get(start_line.saturating_sub(1)..end_line)
        .ok_or_else(|| {
            "memory_search selected canonical snippet window is out of bounds".to_owned()
        })?;
    let snippet = snippet_lines.join("\n");
    let score = best_body_match
        .map(|matched| matched.score)
        .unwrap_or(0)
        .max(metadata_score)
        .max(1);

    Ok(Some(MemorySearchResult {
        source: "canonical_session",
        path: None,
        session_id: Some(hit.record.session_id.clone()),
        scope: Some(hit.record.scope.as_str().to_owned()),
        kind: Some(hit.record.kind.as_str().to_owned()),
        role: hit.record.role.clone(),
        start_line: None,
        end_line: None,
        snippet,
        score,
        provenance: canonical_memory_hit_provenance(config, hit),
        metadata: Some(metadata),
    }))
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
        let root = unique_temp_dir("loong-memory-search-canonical");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");

        crate::memory::append_turn_direct_with_sqlite_path(
            "release-session",
            "assistant",
            "Deployment cutoff is 17:00 Beijing time and requires a release note.",
            &db_path,
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
        let root = unique_temp_dir("loong-memory-search-canonical-metadata");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");

        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();
        crate::memory::append_turn_direct_with_sqlite_path(
            "metadata-session",
            "assistant",
            &payload,
            &db_path,
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
    fn memory_search_tool_matches_workspace_metadata_only_queries() {
        let root = unique_temp_dir("loong-memory-search-workspace-metadata");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(
            root.join("MEMORY.md"),
            concat!(
                "---\n",
                "kind: procedure\n",
                "trust: workspace_curated\n",
                "status: active\n",
                "---\n",
                "发布窗口需要双人复核。\n",
            ),
        )
        .expect("write root memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "procedure",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "results={results:?}");
        assert_eq!(results[0]["source"], "workspace_file");
        assert_eq!(results[0]["path"], "MEMORY.md");
        assert_eq!(results[0]["scope"], "workspace");
        assert_eq!(results[0]["kind"], "procedure");
        assert!(
            results[0]["snippet"]
                .as_str()
                .is_some_and(|value| value.contains("双人复核")),
            "results={results:?}"
        );
    }

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_uses_indexed_workspace_memory_when_sqlite_is_available() {
        let root = unique_temp_dir("loongclaw-memory-search-indexed-workspace");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(root.join("MEMORY.md"), "中文分词用于数据库搜索。\n")
            .expect("write root memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "中文 分词",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "results={results:?}");
        assert_eq!(results[0]["source"], "workspace_file");
        assert_eq!(results[0]["path"], "MEMORY.md");
        assert_eq!(
            results[0]["provenance"]["source_kind"],
            "workspace_document"
        );
        assert_eq!(results[0]["metadata"]["derived_kind"], "overview");
    }

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_matches_indexed_workspace_metadata_only_queries() {
        let root = unique_temp_dir("loong-memory-search-indexed-workspace-metadata");
        let db_path = root.join("memory.sqlite3");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(
            root.join("MEMORY.md"),
            concat!(
                "---\n",
                "kind: procedure\n",
                "trust: workspace_curated\n",
                "status: active\n",
                "---\n",
                "发布窗口需要双人复核。\n",
            ),
        )
        .expect("write root memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "workspace_curated",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "results={results:?}");
        assert_eq!(results[0]["source"], "workspace_file");
        assert_eq!(results[0]["path"], "MEMORY.md");
        assert_eq!(results[0]["scope"], "workspace");
        assert_eq!(results[0]["kind"], "procedure");
        assert_eq!(results[0]["metadata"]["trust_level"], "workspace_curated");
    }

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_refreshes_indexed_workspace_memory_after_file_changes() {
        let root = unique_temp_dir("loongclaw-memory-search-index-refresh");
        let db_path = root.join("memory.sqlite3");
        let memory_path = root.join("MEMORY.md");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(&memory_path, "旧部署窗口记录。\n").expect("write initial memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let first = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "部署 窗口",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("initial memory search should succeed");
        assert_eq!(first.payload["returned"], 1);

        let initial_modified = std::fs::metadata(&memory_path)
            .expect("memory metadata before rewrite")
            .modified()
            .expect("memory modified time before rewrite");
        loop {
            std::fs::write(&memory_path, "新的回滚检查清单。\n").expect("rewrite memory file");
            let rewritten_modified = std::fs::metadata(&memory_path)
                .expect("memory metadata after rewrite")
                .modified()
                .expect("memory modified time after rewrite");
            if rewritten_modified > initial_modified {
                break;
            }
            std::hint::spin_loop();
        }

        let refreshed = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "回滚 检查",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("refreshed memory search should succeed");
        assert_eq!(
            refreshed.payload["returned"], 1,
            "payload={:?}",
            refreshed.payload
        );
        assert!(
            refreshed.payload["results"][0]["snippet"]
                .as_str()
                .is_some_and(|value| value.contains("回滚检查清单")),
            "payload={:?}",
            refreshed.payload
        );

        let stale = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "部署 窗口",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("stale query memory search should succeed");
        assert_eq!(stale.payload["returned"], 0, "payload={:?}", stale.payload);
    }

    #[cfg(all(feature = "tool-file", feature = "memory-sqlite"))]
    #[test]
    fn memory_search_tool_drops_deleted_workspace_documents_from_index() {
        let root = unique_temp_dir("loongclaw-memory-search-index-delete");
        let db_path = root.join("memory.sqlite3");
        let memory_path = root.join("MEMORY.md");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(&memory_path, "发布清单需要双人复核。\n").expect("write memory file");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            memory_sqlite_path: Some(db_path),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let initial = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "发布 清单",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("initial memory search should succeed");
        assert_eq!(
            initial.payload["returned"], 1,
            "payload={:?}",
            initial.payload
        );

        std::fs::remove_file(&memory_path).expect("delete memory file");

        let after_delete = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "发布 清单",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("post-delete memory search should succeed");
        assert_eq!(
            after_delete.payload["returned"], 0,
            "payload={:?}",
            after_delete.payload
        );
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn memory_search_tool_skips_tombstoned_workspace_memory_files() {
        let root = unique_temp_dir("loong-memory-search-tombstoned");
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
    fn memory_search_tool_matches_segmented_chinese_workspace_queries() {
        let root = unique_temp_dir("loongclaw-memory-search-chinese-workspace");

        std::fs::create_dir_all(&root).expect("create root dir");
        std::fs::write(root.join("MEMORY.md"), "中文分词用于数据库搜索。\n")
            .expect("write root memory");

        let config = super::super::runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = super::super::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "中文 分词",
                    "max_results": 4
                }),
            },
            &config,
        )
        .expect("memory search should succeed");

        let results = outcome.payload["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "results={results:?}");
        assert_eq!(results[0]["source"], "workspace_file");
        assert_eq!(results[0]["path"], "MEMORY.md");
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn memory_get_tool_strips_frontmatter_and_surfaces_workspace_metadata() {
        let root = unique_temp_dir("loong-memory-get-frontmatter");
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
