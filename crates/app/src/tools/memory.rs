use loongclaw_contracts::ToolCoreOutcome;
use serde_json::{json, Value};

#[cfg(feature = "memory-sqlite")]
use crate::config::SessionVisibility;
use crate::config::ToolConfig;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::SessionRepository;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemorySearchRequest {
    query: String,
    session_id: Option<String>,
    session_ids: Option<Vec<String>>,
    limit: usize,
    excerpt_chars: usize,
}

pub(crate) fn execute_memory_search_tool_with_policies(
    payload: Value,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (payload, current_session_id, memory_config, tool_config);
        return Err(
            "memory_search requires sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: session tools are disabled by config".to_owned());
        }

        let request = parse_memory_search_request(payload)?;
        let repo = SessionRepository::new(memory_config)?;
        let (mode, searched_session_ids, skipped_targets) =
            resolve_search_scope(&repo, current_session_id, &request, tool_config)?;

        let matches = crate::memory::search_transcript_direct(
            &searched_session_ids,
            &request.query,
            request.limit,
            request.excerpt_chars,
            memory_config,
        )?;
        let mut payload = json!({
            "query": request.query,
            "scope": {
                "mode": mode,
                "current_session_id": current_session_id,
                "searched_session_ids": searched_session_ids,
                "searched_session_count": searched_session_ids.len(),
                "skipped_targets": skipped_targets,
            },
            "matches": matches
                .into_iter()
                .map(|item| {
                    json!({
                        "session_id": item.session_id,
                        "turn_id": item.turn_id,
                        "role": item.role,
                        "ts": item.ts,
                        "content_snippet": item.content_snippet,
                        "match": {
                            "query": request.query,
                            "match_kind": "substring",
                            "excerpt_chars": request.excerpt_chars.clamp(40, 400),
                        }
                    })
                })
                .collect::<Vec<_>>(),
            "returned_count": 0,
            "limit": request.limit.clamp(1, 100),
            "truncated": false,
        });
        let returned_count = payload
            .get("matches")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();

        if let Some(object) = payload.as_object_mut() {
            object.insert("returned_count".to_owned(), json!(returned_count));
        }

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_memory_search_request(payload: Value) -> Result<MemorySearchRequest, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "memory_search_invalid_request: payload must be an object".to_owned())?;
    let query = object
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory_search_invalid_request: query is required".to_owned())?
        .to_owned();
    let session_id = object
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let session_ids = object
        .get("session_ids")
        .map(parse_session_ids)
        .transpose()?;
    if session_id.is_some() && session_ids.is_some() {
        return Err(
            "memory_search_invalid_request: session_id and session_ids are mutually exclusive"
                .to_owned(),
        );
    }

    Ok(MemorySearchRequest {
        query,
        session_id,
        session_ids,
        limit: object.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
        excerpt_chars: object
            .get("excerpt_chars")
            .and_then(Value::as_u64)
            .unwrap_or(120) as usize,
    })
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_ids(value: &Value) -> Result<Vec<String>, String> {
    let items = value.as_array().ok_or_else(|| {
        "memory_search_invalid_request: session_ids must be an array of strings".to_owned()
    })?;
    if items.is_empty() {
        return Err("memory_search_invalid_request: session_ids cannot be empty".to_owned());
    }

    let mut session_ids = Vec::with_capacity(items.len());
    for item in items {
        let session_id = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "memory_search_invalid_request: session_ids must be non-empty strings".to_owned()
            })?;
        session_ids.push(session_id.to_owned());
    }
    Ok(session_ids)
}

#[cfg(feature = "memory-sqlite")]
fn resolve_search_scope(
    repo: &SessionRepository,
    current_session_id: &str,
    request: &MemorySearchRequest,
    tool_config: &ToolConfig,
) -> Result<(&'static str, Vec<String>, Vec<Value>), String> {
    if let Some(session_id) = &request.session_id {
        ensure_target_visible(
            repo,
            current_session_id,
            session_id,
            tool_config.sessions.visibility,
        )?;
        return Ok(("single", vec![session_id.clone()], Vec::new()));
    }

    if let Some(session_ids) = &request.session_ids {
        let mut visible = Vec::new();
        let mut skipped = Vec::new();
        for session_id in session_ids {
            match classify_target_visibility(
                repo,
                current_session_id,
                session_id,
                tool_config.sessions.visibility,
            )? {
                TargetVisibility::Visible => visible.push(session_id.clone()),
                TargetVisibility::NotVisible(message) => skipped.push(json!({
                    "session_id": session_id,
                    "result": "skipped_not_visible",
                    "message": message,
                })),
                TargetVisibility::NotFound(message) => skipped.push(json!({
                    "session_id": session_id,
                    "result": "skipped_not_found",
                    "message": message,
                })),
            }
        }
        return Ok(("batch", visible, skipped));
    }

    let visible_sessions = repo.list_visible_sessions(current_session_id)?;
    Ok((
        "visible",
        visible_sessions
            .into_iter()
            .map(|session| session.session_id)
            .collect(),
        Vec::new(),
    ))
}

#[cfg(feature = "memory-sqlite")]
fn ensure_target_visible(
    repo: &SessionRepository,
    current_session_id: &str,
    target_session_id: &str,
    visibility: SessionVisibility,
) -> Result<(), String> {
    match classify_target_visibility(repo, current_session_id, target_session_id, visibility)? {
        TargetVisibility::Visible => Ok(()),
        TargetVisibility::NotVisible(message) => Err(message),
        TargetVisibility::NotFound(message) => Err(message),
    }
}

#[cfg(feature = "memory-sqlite")]
enum TargetVisibility {
    Visible,
    NotVisible(String),
    NotFound(String),
}

#[cfg(feature = "memory-sqlite")]
fn classify_target_visibility(
    repo: &SessionRepository,
    current_session_id: &str,
    target_session_id: &str,
    visibility: SessionVisibility,
) -> Result<TargetVisibility, String> {
    let summary = repo.load_session_summary_with_legacy_fallback(target_session_id)?;
    if summary.is_none() {
        return Ok(TargetVisibility::NotFound(format!(
            "session_not_found: `{target_session_id}`"
        )));
    }
    if current_session_id == target_session_id {
        return Ok(TargetVisibility::Visible);
    }

    let is_visible = match visibility {
        SessionVisibility::SelfOnly => false,
        SessionVisibility::Children => {
            repo.is_session_visible(current_session_id, target_session_id)?
        }
    };
    if is_visible {
        Ok(TargetVisibility::Visible)
    } else {
        Ok(TargetVisibility::NotVisible(format!(
            "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use loongclaw_contracts::ToolCoreRequest;
    use serde_json::{json, Value};

    use crate::config::ToolConfig;
    use crate::memory::append_turn_direct;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        FinalizeSessionTerminalRequest, NewSessionEvent, NewSessionRecord, SessionKind,
        SessionRepository, SessionState, TransitionSessionWithEventIfCurrentRequest,
    };

    use super::super::execute_app_tool_with_config;

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-memory-tool-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
        }
    }

    fn skipped_target<'a>(payload: &'a Value, session_id: &str) -> &'a Value {
        payload
            .get("scope")
            .and_then(|scope| scope.get("skipped_targets"))
            .and_then(Value::as_array)
            .expect("skipped_targets array")
            .iter()
            .find(|item| item.get("session_id").and_then(Value::as_str) == Some(session_id))
            .unwrap_or_else(|| panic!("missing skipped target `{session_id}`"))
    }

    fn archive_completed_session(
        repo: &SessionRepository,
        session_id: &str,
        actor_session_id: &str,
    ) {
        repo.finalize_session_terminal(
            session_id,
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some(actor_session_id.to_owned()),
                event_payload_json: json!({}),
                outcome_status: "completed".to_owned(),
                outcome_payload_json: json!({}),
            },
        )
        .expect("finalize completed");
        repo.transition_session_with_event_if_current(
            session_id,
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Completed,
                next_state: SessionState::Completed,
                last_error: None,
                event_kind: "session_archived".to_owned(),
                actor_session_id: Some(actor_session_id.to_owned()),
                event_payload_json: json!({
                    "previous_state": "completed",
                    "hides_from_sessions_list": true
                }),
            },
        )
        .expect("archive session")
        .expect("archive transition result");
    }

    #[test]
    fn memory_search_searches_visible_scope_by_default() {
        let config = isolated_memory_config("visible-scope");
        let repo = SessionRepository::new(&config).expect("repository");
        let tool_config = ToolConfig::default();

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
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "other-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Other".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create other root");

        append_turn_direct("root-session", "user", "timeout budget root", &config)
            .expect("append root turn");
        append_turn_direct(
            "child-session",
            "assistant",
            "timeout budget child",
            &config,
        )
        .expect("append child turn");
        append_turn_direct("other-root", "assistant", "timeout budget hidden", &config)
            .expect("append hidden turn");

        let outcome = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "timeout budget"
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect("memory_search outcome");

        let scope = outcome.payload.get("scope").expect("scope payload");
        assert_eq!(scope.get("mode").and_then(Value::as_str), Some("visible"));
        assert_eq!(
            outcome
                .payload
                .get("returned_count")
                .and_then(Value::as_u64),
            Some(2)
        );
        let matches = outcome
            .payload
            .get("matches")
            .and_then(Value::as_array)
            .expect("matches array");
        let session_ids: Vec<&str> = matches
            .iter()
            .filter_map(|item| item.get("session_id"))
            .filter_map(Value::as_str)
            .collect();
        assert!(session_ids.contains(&"root-session"));
        assert!(session_ids.contains(&"child-session"));
        assert!(!session_ids.contains(&"other-root"));
    }

    #[test]
    fn memory_search_rejects_invisible_single_target() {
        let config = isolated_memory_config("single-target-visibility");
        let repo = SessionRepository::new(&config).expect("repository");
        let tool_config = ToolConfig::default();

        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "other-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Other".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create other");

        let error = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "timeout budget",
                    "session_id": "other-root"
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect_err("hidden single target should fail");

        assert!(
            error.contains("visibility_denied"),
            "expected visibility_denied, got: {error}"
        );
    }

    #[test]
    fn memory_search_batch_reports_skipped_targets() {
        let config = isolated_memory_config("batch-skips");
        let repo = SessionRepository::new(&config).expect("repository");
        let tool_config = ToolConfig::default();

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
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden");

        append_turn_direct(
            "child-session",
            "assistant",
            "timeout budget child",
            &config,
        )
        .expect("append child turn");

        let outcome = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "timeout budget",
                    "session_ids": ["child-session", "hidden-root", "missing-session"]
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect("batch memory_search outcome");

        let scope = outcome.payload.get("scope").expect("scope payload");
        assert_eq!(scope.get("mode").and_then(Value::as_str), Some("batch"));
        assert_eq!(
            outcome
                .payload
                .get("returned_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            skipped_target(&outcome.payload, "hidden-root")
                .get("result")
                .and_then(Value::as_str),
            Some("skipped_not_visible")
        );
        assert_eq!(
            skipped_target(&outcome.payload, "missing-session")
                .get("result")
                .and_then(Value::as_str),
            Some("skipped_not_found")
        );
    }

    #[test]
    fn memory_search_includes_archived_visible_sessions() {
        let config = isolated_memory_config("archived-visible");
        let repo = SessionRepository::new(&config).expect("repository");
        let tool_config = ToolConfig::default();

        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "archived-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Archived".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create archived child");
        append_turn_direct(
            "archived-child",
            "assistant",
            "restore inventory marker",
            &config,
        )
        .expect("append archived turn");
        archive_completed_session(&repo, "archived-child", "root-session");

        let outcome = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "restore inventory marker"
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect("memory_search outcome");

        let matches = outcome
            .payload
            .get("matches")
            .and_then(Value::as_array)
            .expect("matches array");
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches
                .first()
                .and_then(|item| item.get("session_id"))
                .and_then(Value::as_str),
            Some("archived-child")
        );
    }

    #[test]
    fn memory_search_supports_legacy_current_session_transcript() {
        let config = isolated_memory_config("legacy-current");
        let tool_config = ToolConfig::default();

        append_turn_direct(
            "delegate:legacy-child",
            "assistant",
            "legacy timeout budget note",
            &config,
        )
        .expect("append legacy turn");

        let outcome = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "timeout budget"
                }),
            },
            "delegate:legacy-child",
            &config,
            &tool_config,
        )
        .expect("legacy memory_search outcome");

        assert_eq!(
            outcome
                .payload
                .get("returned_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            outcome
                .payload
                .get("matches")
                .and_then(Value::as_array)
                .and_then(|matches| matches.first())
                .and_then(|item| item.get("session_id"))
                .and_then(Value::as_str),
            Some("delegate:legacy-child")
        );
    }

    #[test]
    fn memory_search_does_not_return_session_events() {
        let config = isolated_memory_config("ignores-events");
        let repo = SessionRepository::new(&config).expect("repository");
        let tool_config = ToolConfig::default();

        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "delegate_progress".to_owned(),
            actor_session_id: None,
            payload_json: json!({
                "note": "timeout budget appears only in control-plane event"
            }),
        })
        .expect("append control event");

        let outcome = execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "memory_search".to_owned(),
                payload: json!({
                    "query": "timeout budget"
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect("memory_search outcome");

        let matches = outcome
            .payload
            .get("matches")
            .and_then(Value::as_array)
            .expect("matches array");
        assert!(
            matches.is_empty(),
            "session events should not be searchable transcript hits"
        );
    }
}
