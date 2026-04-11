#[cfg(feature = "memory-sqlite")]
use std::collections::{BTreeMap, BTreeSet};

use loongclaw_contracts::ToolCoreOutcome;
use serde_json::{Value, json};

use super::payload::{optional_payload_limit, optional_payload_string, required_payload_string};

use crate::config::{SessionVisibility, ToolConfig};
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::session::repository::{SessionRepository, SessionSearchSourceKind};

const DEFAULT_SESSION_SEARCH_MAX_RESULTS: usize = 5;
const MAX_SESSION_SEARCH_MAX_RESULTS: usize = 20;
const SESSION_SEARCH_PER_SESSION_LIMIT_CAP: usize = 10;
const SESSION_SEARCH_SNIPPET_BEFORE_CHARS: usize = 80;
const SESSION_SEARCH_SNIPPET_AFTER_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSearchHit {
    session_id: String,
    label: Option<String>,
    session_state: String,
    archived: bool,
    source: SessionSearchSourceKind,
    source_id: i64,
    role: Option<String>,
    event_kind: Option<String>,
    ts: i64,
    snippet: String,
    score: u32,
}

pub(super) fn execute_session_search_with_policies(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let query = required_payload_string(&payload, "query", "session_search")?;
    let max_results = optional_payload_limit(
        &payload,
        "max_results",
        DEFAULT_SESSION_SEARCH_MAX_RESULTS,
        MAX_SESSION_SEARCH_MAX_RESULTS,
    );
    let include_archived = optional_payload_bool(&payload, "include_archived", false)?;
    let include_turns = optional_payload_bool(&payload, "include_turns", true)?;
    let include_events = optional_payload_bool(&payload, "include_events", true)?;
    if !include_turns && !include_events {
        return Err(
            "session_search requires at least one enabled source: include_turns or include_events"
                .to_owned(),
        );
    }

    let target_session_id = optional_payload_string(&payload, "session_id");
    let repo = SessionRepository::new(config)?;
    let mut visible_sessions = repo.list_visible_sessions(current_session_id)?;
    if tool_config.sessions.visibility == SessionVisibility::SelfOnly {
        visible_sessions.retain(|session| session.session_id == current_session_id);
    }
    if !include_archived {
        visible_sessions.retain(|session| session.archived_at.is_none());
    }

    if let Some(target_session_id) = target_session_id.as_deref() {
        ensure_session_search_target_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;

        let archived_hidden = !include_archived
            && visible_sessions
                .iter()
                .all(|session| session.session_id != target_session_id);
        if archived_hidden {
            return Err(format!(
                "session_search target session `{target_session_id}` is archived; set include_archived=true to search archived sessions"
            ));
        }

        visible_sessions.retain(|session| session.session_id == target_session_id);
    }

    let searched_session_count = visible_sessions.len();
    if searched_session_count == 0 {
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "current_session_id": current_session_id,
                "query": query,
                "returned_count": 0,
                "matched_session_count": 0,
                "searched_session_count": 0,
                "include_archived": include_archived,
                "include_turns": include_turns,
                "include_events": include_events,
                "results": [],
            }),
        });
    }

    let normalized_query = query.to_ascii_lowercase();
    let query_tokens = tokenize_search_query(normalized_query.as_str());
    let per_session_limit = max_results
        .min(SESSION_SEARCH_PER_SESSION_LIMIT_CAP)
        .saturating_add(2);
    let session_by_id = visible_sessions
        .into_iter()
        .map(|session| (session.session_id.clone(), session))
        .collect::<BTreeMap<_, _>>();

    let mut hits = Vec::new();
    for session in session_by_id.values() {
        let records = repo.search_session_content(
            session.session_id.as_str(),
            query.as_str(),
            per_session_limit,
        )?;
        for record in records {
            let include_source = match record.source_kind {
                SessionSearchSourceKind::Turn => include_turns,
                SessionSearchSourceKind::Event => include_events,
            };
            if !include_source {
                continue;
            }

            let score = session_search_score(
                normalized_query.as_str(),
                query_tokens.as_slice(),
                record.content_text.as_str(),
            );
            if score == 0 {
                continue;
            }

            let snippet = build_search_snippet(
                record.content_text.as_str(),
                normalized_query.as_str(),
                query_tokens.as_slice(),
            );

            hits.push(SessionSearchHit {
                session_id: session.session_id.clone(),
                label: session.label.clone(),
                session_state: session.state.as_str().to_owned(),
                archived: session.archived_at.is_some(),
                source: record.source_kind,
                source_id: record.source_id,
                role: record.role,
                event_kind: record.event_kind,
                ts: record.ts,
                snippet,
                score,
            });
        }
    }

    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(right.ts.cmp(&left.ts))
            .then(left.session_id.cmp(&right.session_id))
            .then(right.source_id.cmp(&left.source_id))
    });
    hits.truncate(max_results);

    let matched_session_count = hits
        .iter()
        .map(|hit| hit.session_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "query": query,
            "returned_count": hits.len(),
            "matched_session_count": matched_session_count,
            "searched_session_count": searched_session_count,
            "include_archived": include_archived,
            "include_turns": include_turns,
            "include_events": include_events,
            "results": hits.into_iter().map(session_search_hit_json).collect::<Vec<_>>(),
        }),
    })
}

fn optional_payload_bool(payload: &Value, key: &str, default_value: bool) -> Result<bool, String> {
    let Some(raw_value) = payload.get(key) else {
        return Ok(default_value);
    };
    raw_value
        .as_bool()
        .ok_or_else(|| format!("session_search payload.{key} must be a boolean"))
}

fn ensure_session_search_target_visible(
    repo: &SessionRepository,
    current_session_id: &str,
    target_session_id: &str,
    visibility: SessionVisibility,
) -> Result<(), String> {
    let is_visible = match visibility {
        SessionVisibility::SelfOnly => current_session_id == target_session_id,
        SessionVisibility::Children => {
            current_session_id == target_session_id
                || repo.is_session_visible(current_session_id, target_session_id)?
        }
    };
    if is_visible {
        return Ok(());
    }
    Err(format!(
        "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
    ))
}

fn session_search_score(query: &str, query_tokens: &[String], content: &str) -> u32 {
    let normalized_content = content.to_ascii_lowercase();
    let mut score = 0u32;

    if normalized_content.contains(query) {
        score = score.saturating_add(100);
    }

    let mut matched_tokens = 0u32;
    for token in query_tokens {
        if normalized_content.contains(token) {
            matched_tokens = matched_tokens.saturating_add(1);
            score = score.saturating_add(20);
        }
    }

    if matched_tokens > 1 && matched_tokens as usize == query_tokens.len() {
        score = score.saturating_add(20);
    }

    score
}

fn tokenize_search_query(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn build_search_snippet(content: &str, query: &str, query_tokens: &[String]) -> String {
    let normalized_content = content.to_ascii_lowercase();
    let match_index = normalized_content
        .find(query)
        .or_else(|| {
            query_tokens
                .iter()
                .find_map(|token| normalized_content.find(token.as_str()))
        })
        .unwrap_or(0);

    let match_char_index = normalized_content[..match_index].chars().count();
    let total_chars = content.chars().count();
    let start_char = match_char_index.saturating_sub(SESSION_SEARCH_SNIPPET_BEFORE_CHARS);
    let end_char = match_char_index
        .saturating_add(SESSION_SEARCH_SNIPPET_AFTER_CHARS)
        .min(total_chars);
    let mut snippet = content
        .chars()
        .skip(start_char)
        .take(end_char.saturating_sub(start_char))
        .collect::<String>();

    if start_char > 0 {
        snippet = format!("...{snippet}");
    }
    if end_char < total_chars {
        snippet.push_str("...");
    }

    snippet
}

fn session_search_hit_json(hit: SessionSearchHit) -> Value {
    json!({
        "session_id": hit.session_id,
        "label": hit.label,
        "session_state": hit.session_state,
        "archived": hit.archived,
        "source": hit.source.as_str(),
        "source_id": hit.source_id,
        "role": hit.role,
        "event_kind": hit.event_kind,
        "ts": hit.ts,
        "snippet": hit.snippet,
        "score": hit.score,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::memory::append_turn_direct;
    use crate::session::repository::{
        FinalizeSessionTerminalRequest, NewSessionEvent, NewSessionRecord, SessionKind,
        SessionRepository, SessionState,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-session-search-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn create_root_and_child(repo: &SessionRepository) {
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "other-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Other".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create other");
    }

    #[test]
    fn session_search_returns_visible_turn_and_event_hits() {
        let config = isolated_memory_config("returns-visible-hits");
        let repo = SessionRepository::new(&config).expect("repository");
        create_root_and_child(&repo);

        append_turn_direct(
            "child-session",
            "assistant",
            "Deploy freeze window is Friday and customer migration starts Saturday.",
            &config,
        )
        .expect("append child turn");
        append_turn_direct(
            "other-session",
            "assistant",
            "Deploy freeze for hidden session.",
            &config,
        )
        .expect("append hidden turn");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "summary": "deploy freeze checklist completed"
            }),
        })
        .expect("append child event");

        let outcome = execute_session_search_with_policies(
            json!({
                "query": "deploy freeze",
                "max_results": 6
            }),
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("session_search outcome");

        let results = outcome.payload["results"].as_array().expect("results");
        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|entry| entry["session_id"] != "other-session"),
            "hidden sessions must not leak into visible search results: {results:?}"
        );
        assert!(
            results
                .iter()
                .any(|entry| entry["source"] == "turn" && entry["session_id"] == "child-session")
        );
        assert!(
            results
                .iter()
                .any(|entry| entry["source"] == "event" && entry["session_id"] == "child-session")
        );
    }

    #[test]
    fn session_search_respects_self_only_visibility() {
        let config = isolated_memory_config("self-only");
        let repo = SessionRepository::new(&config).expect("repository");
        create_root_and_child(&repo);

        append_turn_direct(
            "child-session",
            "assistant",
            "Deploy freeze window is Friday.",
            &config,
        )
        .expect("append child turn");

        let mut tool_config = ToolConfig::default();
        tool_config.sessions.visibility = SessionVisibility::SelfOnly;

        let outcome = execute_session_search_with_policies(
            json!({
                "query": "deploy freeze"
            }),
            "root-session",
            &config,
            &tool_config,
        )
        .expect("session_search outcome");

        assert_eq!(outcome.payload["returned_count"], 0);
        assert_eq!(outcome.payload["matched_session_count"], 0);
    }

    #[test]
    fn session_search_can_include_archived_target_when_requested() {
        let config = isolated_memory_config("include-archived");
        let repo = SessionRepository::new(&config).expect("repository");
        create_root_and_child(&repo);

        append_turn_direct(
            "child-session",
            "assistant",
            "Deploy freeze window is Friday.",
            &config,
        )
        .expect("append child turn");
        repo.finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": "child-session" }),
                frozen_result: None,
            },
        )
        .expect("finalize child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "session_archived".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "reason": "test" }),
        })
        .expect("archive child");

        let hidden_error = execute_session_search_with_policies(
            json!({
                "query": "deploy freeze",
                "session_id": "child-session"
            }),
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("archived session should be hidden by default");
        assert!(hidden_error.contains("include_archived=true"));

        let visible_outcome = execute_session_search_with_policies(
            json!({
                "query": "deploy freeze",
                "session_id": "child-session",
                "include_archived": true
            }),
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("archived search outcome");
        assert_eq!(visible_outcome.payload["returned_count"], 1);
        assert_eq!(visible_outcome.payload["results"][0]["archived"], true);
    }
}
