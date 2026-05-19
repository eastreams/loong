use super::*;

pub(super) fn execute_sessions_list(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_sessions_list_request(&payload, tool_config)?;
    let include_delegate_lifecycle = request.effective_include_delegate_lifecycle();
    let now_ts = current_unix_ts();
    let mut sessions = repo.list_visible_sessions(current_session_id)?;
    if tool_config.sessions.visibility == SessionVisibility::SelfOnly {
        sessions.retain(|session| session.session_id == current_session_id);
    }
    if let Some(state) = request.state {
        sessions.retain(|session| session.state == state);
    }
    if let Some(kind) = request.kind {
        sessions.retain(|session| session.kind == kind);
    }
    if let Some(parent_session_id) = request.parent_session_id.as_deref() {
        sessions.retain(|session| session.parent_session_id.as_deref() == Some(parent_session_id));
    }
    if !request.include_archived {
        sessions.retain(|session| session.archived_at.is_none());
    }

    let mut listed_sessions = Vec::new();
    for session in sessions {
        let delegate_events = if session.kind == SessionKind::DelegateChild {
            Some(load_delegate_lifecycle_events(&repo, &session)?)
        } else {
            None
        };
        let delegate_lifecycle = delegate_events
            .as_deref()
            .and_then(|events| session_delegate_lifecycle_at(&session, events, now_ts));
        let subagent_contract = resolve_subagent_contract_for_session(
            &repo,
            &session,
            delegate_lifecycle.as_ref(),
            tool_config,
        )?;
        if request.overdue_only
            && !delegate_lifecycle
                .as_ref()
                .and_then(|lifecycle| lifecycle.staleness.as_ref())
                .map(|staleness| staleness.state == "overdue")
                .unwrap_or(false)
        {
            continue;
        }
        listed_sessions.push((
            session,
            delegate_events,
            delegate_lifecycle,
            subagent_contract,
        ));
    }

    let matched_count = listed_sessions.len();
    let effective_offset = request.offset.min(matched_count);
    let page_limit = request.limit.saturating_add(1);
    let visible_sessions = listed_sessions.into_iter();
    let offset_sessions = visible_sessions.skip(effective_offset);
    let bounded_sessions = offset_sessions.take(page_limit);
    let mut listed_sessions = bounded_sessions.collect::<Vec<_>>();
    let has_more = listed_sessions.len() > request.limit;
    if has_more {
        let _ = listed_sessions.pop();
    }
    let returned_count = listed_sessions.len();
    let mut session_payloads = Vec::with_capacity(returned_count);
    for (session, delegate_events, delegate_lifecycle, subagent_contract) in listed_sessions {
        let workflow = load_session_workflow_record(&repo, &session, delegate_events.as_deref())?;
        let payload = session_summary_json_with_delegate_lifecycle(
            session,
            workflow,
            delegate_lifecycle,
            subagent_contract,
            include_delegate_lifecycle,
        );
        session_payloads.push(payload);
    }
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filters": sessions_list_filters_json(&request),
            "matched_count": matched_count,
            "returned_count": returned_count,
            "has_more": has_more,
            "sessions": session_payloads,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_tasks_list(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_tasks_list_request(&payload, tool_config);
    let visible_tasks = load_visible_task_records(&repo, current_session_id)?;

    let mut tasks = Vec::new();
    for visible_task in visible_tasks {
        let task_progress = visible_task.task_progress;
        let task_state = task_progress.status.as_str().to_owned();
        let task_is_stable = task_progress.status.is_stable();
        let state_filter = request.task_state.as_deref();
        let matches_state = state_filter.is_none_or(|expected| expected == task_state.as_str());
        if !matches_state {
            continue;
        }
        if request.stable_only && !task_is_stable {
            continue;
        }

        let verification_state = task_progress
            .verification_state
            .map(|value| value.as_str().to_owned());
        tasks.push(json!({
            "task_id": visible_task.task_id,
            "task_state": task_state,
            "task_is_stable": task_is_stable,
            "intent_summary": task_progress.intent_summary,
            "verification_state": verification_state,
            "owner_session_id": visible_task.owner_session_id,
            "session_label": visible_task.session_label,
            "updated_at": task_progress.updated_at,
            "active_handles": task_progress.active_handles,
            "resume_recipe": task_progress.resume_recipe,
        }));
    }

    let matched_count = tasks.len();
    let effective_offset = request.offset.min(matched_count);
    let mut tasks = tasks
        .into_iter()
        .skip(effective_offset)
        .take(request.limit.saturating_add(1))
        .collect::<Vec<_>>();
    let has_more = tasks.len() > request.limit;
    if has_more {
        let _ = tasks.pop();
    }
    let returned_count = tasks.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "tasks_list",
            "current_session_id": current_session_id,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "has_more": has_more,
            "filters": {
                "task_state": request.task_state,
                "stable_only": request.stable_only,
                "limit": request.limit,
                "offset": request.offset,
            },
            "tasks": tasks,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_tasks_search(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_tasks_search_request(&payload, tool_config)?;
    let visible_tasks = load_visible_task_records(&repo, current_session_id)?;
    let query = request.query.to_ascii_lowercase();

    let mut tasks = Vec::new();
    for visible_task in visible_tasks {
        let task_progress = visible_task.task_progress;
        let task_state = task_progress.status.as_str().to_owned();
        let task_is_stable = task_progress.status.is_stable();
        let state_filter = request.task_state.as_deref();
        let matches_state = state_filter.is_none_or(|expected| expected == task_state.as_str());
        if !matches_state {
            continue;
        }
        if request.stable_only && !task_is_stable {
            continue;
        }

        let session_label = visible_task.session_label.as_deref().unwrap_or_default();
        let haystack = [
            visible_task.task_id.as_str(),
            visible_task.owner_session_id.as_str(),
            task_state.as_str(),
            task_progress.intent_summary.as_deref().unwrap_or_default(),
            session_label,
            task_progress.owner_kind.as_str(),
        ]
        .join(" ")
        .to_ascii_lowercase();

        if !haystack.contains(query.as_str()) {
            continue;
        }

        let verification_state = task_progress
            .verification_state
            .map(|value| value.as_str().to_owned());
        tasks.push(json!({
            "task_id": visible_task.task_id,
            "task_state": task_state,
            "task_is_stable": task_is_stable,
            "intent_summary": task_progress.intent_summary,
            "verification_state": verification_state,
            "owner_session_id": visible_task.owner_session_id,
            "session_label": visible_task.session_label,
            "updated_at": task_progress.updated_at,
            "active_handles": task_progress.active_handles,
            "resume_recipe": task_progress.resume_recipe,
        }));
    }

    let matched_count = tasks.len();
    let tasks = tasks
        .into_iter()
        .take(request.max_results)
        .collect::<Vec<_>>();
    let returned_count = tasks.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "tasks_search",
            "current_session_id": current_session_id,
            "query": request.query,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "filters": {
                "task_state": request.task_state,
                "stable_only": request.stable_only,
                "max_results": request.max_results,
            },
            "tasks": tasks,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_events(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let events = match after_id {
        Some(after_id) => repo.list_events_after(&target_session_id, after_id.max(0), limit)?,
        None => repo.list_recent_events(&target_session_id, limit)?,
    };
    let next_after_id = events
        .last()
        .map(|event| event.id)
        .unwrap_or(after_id.unwrap_or(0));

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "session_id": target_session_id,
            "after_id": after_id,
            "limit": limit,
            "next_after_id": next_after_id,
            "events": events.into_iter().map(session_event_json).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_sessions_history(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let turns = store::window_session_turns(&target_session_id, limit, config)
        .map_err(|error| format!("load session transcript failed: {error}"))?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "session_id": target_session_id,
            "limit": limit,
            "turns": turns,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_task_history(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let lineage_records = load_task_lineage_records(&repo, current_session_id, &resolved_target)?;
    let current_owner_session_id = resolved_target.owner_session_id.as_str();
    let current_task_session_id = lineage_records
        .iter()
        .find(|lineage_record| lineage_record.owner_session_id == current_owner_session_id)
        .map(|lineage_record| lineage_record.task_session_id.clone())
        .unwrap_or_else(|| resolved_target.owner_session_id.clone());
    let turns = load_task_history_turns(
        config,
        lineage_records.as_slice(),
        current_owner_session_id,
        limit,
    )?;
    let task_events = load_task_history_events(
        &repo,
        lineage_records.as_slice(),
        current_owner_session_id,
        None,
        limit,
    )?;
    let task_sessions = lineage_records
        .iter()
        .map(|lineage_record| task_session_summary_json(lineage_record, current_owner_session_id))
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "task_history",
            "task_id": resolved_target.task_id,
            "owner_session_id": resolved_target.owner_session_id,
            "task_session_id": current_task_session_id,
            "lineage_session_count": lineage_records.len(),
            "limit": limit,
            "task_sessions": task_sessions,
            "turns": turns,
            "task_events": task_events,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_task_events(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let lineage_records = load_task_lineage_records(&repo, current_session_id, &resolved_target)?;
    let current_owner_session_id = resolved_target.owner_session_id.as_str();
    let current_task_session_id = lineage_records
        .iter()
        .find(|lineage_record| lineage_record.owner_session_id == current_owner_session_id)
        .map(|lineage_record| lineage_record.task_session_id.clone())
        .unwrap_or_else(|| resolved_target.owner_session_id.clone());
    let (events, next_after_id) = load_task_event_window(
        &repo,
        lineage_records.as_slice(),
        current_owner_session_id,
        after_id,
        limit,
    )?;
    let task_sessions = lineage_records
        .iter()
        .map(|lineage_record| task_session_summary_json(lineage_record, current_owner_session_id))
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "task_events",
            "task_id": resolved_target.task_id,
            "owner_session_id": resolved_target.owner_session_id,
            "task_session_id": current_task_session_id,
            "task_session_count": lineage_records.len(),
            "after_id": after_id,
            "next_after_id": next_after_id,
            "limit": limit,
            "task_sessions": task_sessions,
            "events": events,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_node_payload(node: &SessionNodeRecord) -> Value {
    json!({
        "session_id": node.session_id,
        "node_id": node.node_id,
        "parent_node_id": node.parent_node_id,
        "kind": node.kind.as_str(),
        "role": node.role,
        "content": node.content,
        "session_turn_index": node.session_turn_index,
        "metadata": node.metadata_json,
        "created_at": node.created_at,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_head_payload(head: &SessionHeadRecord) -> Value {
    json!({
        "session_id": head.session_id,
        "head_name": head.head_name,
        "node_id": head.node_id,
        "head_mode": head.mode.as_str(),
        "updated_at": head.updated_at,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_artifact_payload(artifact: &SessionArtifactRecord) -> Value {
    json!({
        "artifact_id": artifact.artifact_id,
        "session_id": artifact.session_id,
        "kind": artifact.kind.as_str(),
        "head_name": artifact.head_name,
        "anchor_node_id": artifact.anchor_node_id,
        "source_start_node_id": artifact.source_start_node_id,
        "source_end_node_id": artifact.source_end_node_id,
        "summary_text": artifact.summary_text,
        "payload": artifact.payload_json,
        "created_at": artifact.created_at,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_heads(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let heads = repo.list_session_heads(&target_session_id)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_heads",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "head_count": heads.len(),
            "heads": heads.iter().map(session_head_payload).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_path(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let head_name =
        optional_payload_string(&payload, "head_name").unwrap_or_else(|| "active".to_owned());
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let path = repo.load_session_path_for_head(&target_session_id, &head_name)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_path",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "head_name": head_name,
            "node_count": path.len(),
            "path": path.iter().map(session_node_payload).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_children(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let parent_node_id = required_payload_string(&payload, "node_id", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let children = repo.list_session_node_children(&target_session_id, &parent_node_id)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_children",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "node_id": parent_node_id,
            "child_count": children.len(),
            "children": children.iter().map(session_node_payload).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_artifacts(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let artifacts = repo.list_session_artifacts(&target_session_id)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_artifacts",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "artifact_count": artifacts.len(),
            "artifacts": artifacts.iter().map(session_artifact_payload).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_status(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_target_request(&payload)?;
    if request.legacy_single {
        let target_session_id = legacy_single_session_id(&request.session_ids)?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            5,
        )?;

        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: session_inspection_payload(snapshot),
        });
    }

    let mut results = Vec::with_capacity(request.session_ids.len());
    for target_session_id in &request.session_ids {
        results.push(execute_session_status_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload_without_dry_run(
            "session_status",
            current_session_id,
            request.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_task_status(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_task_target_request(&payload, "task_id", Some("task_ids"))?;
    let resolved_targets =
        resolve_task_targets(&repo, current_session_id, &request.task_ids, tool_config)?;

    if request.legacy_single {
        let resolved_target = legacy_single_task_target(&resolved_targets)?;
        let snapshot = inspect_visible_session_with_policies(
            &resolved_target.owner_session_id,
            current_session_id,
            config,
            tool_config,
            5,
        )?;
        let lineage_records =
            load_task_lineage_records(&repo, current_session_id, resolved_target)?;
        let payload = session_inspection_payload(snapshot);
        let task_state = task_state_from_payload(&payload);
        let payload = rewrite_task_payload_aliases(payload, "task_status");
        let payload = decorate_task_status_payload(payload, task_state);
        let payload = decorate_task_lineage_payload(
            payload,
            lineage_records.as_slice(),
            &resolved_target.owner_session_id,
        );

        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(resolved_targets.len());
    for resolved_target in &resolved_targets {
        results.push(execute_task_status_batch_result(
            resolved_target,
            current_session_id,
            config,
            tool_config,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: rewrite_task_payload_aliases(
            session_batch_payload_without_dry_run(
                "task_status",
                current_session_id,
                resolved_targets.len(),
                results,
            ),
            "task_status",
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_task_status_batch_result(
    resolved_target: &ResolvedTaskTarget,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    let owner_session_id = resolved_target.owner_session_id.as_str();
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        owner_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            resolved_target.owner_session_id.clone(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        owner_session_id,
        current_session_id,
        config,
        tool_config,
        5,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                resolved_target.owner_session_id.clone(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let lineage_records = load_task_lineage_records(&repo, current_session_id, resolved_target)?;
    let payload = session_inspection_payload(snapshot);
    let task_state = task_state_from_payload(&payload);
    let payload = rewrite_task_payload_aliases(payload, "task_status");
    let payload = decorate_task_status_payload(payload, task_state);
    let payload =
        decorate_task_lineage_payload(payload, lineage_records.as_slice(), owner_session_id);

    Ok(session_batch_result(
        resolved_target.owner_session_id.clone(),
        "ok",
        None,
        None,
        Some(payload),
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn inspect_visible_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    recent_event_limit: usize,
) -> Result<SessionInspectionSnapshot, String> {
    Ok(observe_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        recent_event_limit,
        None,
        0,
    )?
    .inspection)
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
pub(super) async fn wait_for_single_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let started_at = Instant::now();
    let mut next_after_id = after_id.unwrap_or(0).max(0);
    let mut observed_events = Vec::new();
    let mailbox = mailbox_for_session(current_session_id);
    let mut mailbox_subscription = mailbox.subscribe();

    loop {
        let observation = observe_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            event_limit,
            after_id.map(|_| next_after_id),
            event_limit,
        )?;
        let snapshot = observation.inspection;
        if let Some(last_tail_event_id) = observation.tail_events.last().map(|event| event.id) {
            next_after_id = last_tail_event_id;
        }
        observed_events.extend(observation.tail_events);
        if session_state_is_terminal(snapshot.session.state) {
            return Ok(wait_outcome(
                "ok",
                snapshot,
                after_id,
                timeout_ms,
                if after_id.is_some() {
                    observed_events
                } else {
                    Vec::new()
                },
                next_after_id,
            ));
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            return Ok(ToolCoreOutcome {
                status: "timeout".to_owned(),
                payload: wait_payload(
                    snapshot,
                    "timeout",
                    after_id,
                    timeout_ms,
                    if after_id.is_some() {
                        observed_events
                    } else {
                        Vec::new()
                    },
                    next_after_id,
                ),
            });
        }

        let remaining_ms = timeout_ms - elapsed_ms;
        let drained: Vec<InterAgentMessage> = mailbox.drain().await;
        if !drained.is_empty() {
            continue;
        }

        let wait_result = timeout(
            Duration::from_millis(remaining_ms),
            mailbox_subscription.changed(),
        )
        .await;
        if let Ok(Err(_)) = wait_result {
            return Err("session_wait_internal_error: mailbox subscription closed".to_owned());
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
pub(super) async fn wait_for_session_batch_with_policies(
    target_session_ids: Vec<String>,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let mailbox = mailbox_for_session(current_session_id);
    let mut mailbox_subscription = mailbox.subscribe();
    let mut results = vec![None; target_session_ids.len()];
    let mut pending = Vec::new();
    for (index, target_session_id) in target_session_ids.into_iter().enumerate() {
        if let Err(error) = ensure_visible(
            &repo,
            current_session_id,
            &target_session_id,
            tool_config.sessions.visibility,
        ) {
            set_session_batch_result(
                &mut results,
                index,
                session_batch_result(
                    target_session_id,
                    "skipped_not_visible",
                    Some(error),
                    None,
                    None,
                ),
            )?;
            continue;
        }
        pending.push(SessionWaitTargetState {
            index,
            session_id: target_session_id,
            next_after_id: after_id.unwrap_or(0).max(0),
            observed_events: Vec::new(),
            latest_inspection: None,
        });
    }
    drop(repo);

    let started_at = Instant::now();
    loop {
        let mut next_pending = Vec::with_capacity(pending.len());
        for mut target in pending.into_iter() {
            let observation = match observe_visible_session_with_policies(
                &target.session_id,
                current_session_id,
                config,
                tool_config,
                event_limit,
                after_id.map(|_| target.next_after_id),
                event_limit,
            ) {
                Ok(observation) => observation,
                Err(error) if is_session_visibility_skip_error(&error) => {
                    set_session_batch_result(
                        &mut results,
                        target.index,
                        session_batch_result(
                            target.session_id,
                            "skipped_not_visible",
                            Some(error),
                            None,
                            None,
                        ),
                    )?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let snapshot = observation.inspection;
            if let Some(last_tail_event_id) = observation.tail_events.last().map(|event| event.id) {
                target.next_after_id = last_tail_event_id;
            }
            target.observed_events.extend(observation.tail_events);
            target.latest_inspection = Some(snapshot.clone());
            if session_state_is_terminal(snapshot.session.state) {
                set_session_batch_result(
                    &mut results,
                    target.index,
                    session_batch_result(
                        target.session_id,
                        "ok",
                        None,
                        None,
                        Some(wait_payload(
                            snapshot,
                            "completed",
                            after_id,
                            timeout_ms,
                            if after_id.is_some() {
                                std::mem::take(&mut target.observed_events)
                            } else {
                                Vec::new()
                            },
                            target.next_after_id,
                        )),
                    ),
                )?;
                continue;
            }
            next_pending.push(target);
        }
        pending = next_pending;

        if pending.is_empty() {
            let results = collect_session_batch_results(results)?;
            return Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: session_wait_batch_payload(
                    current_session_id,
                    after_id,
                    timeout_ms,
                    results,
                ),
            });
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            for mut target in pending.into_iter() {
                let snapshot = target.latest_inspection.take().ok_or_else(|| {
                    format!(
                        "session_wait_internal_error: missing pending inspection for `{}`",
                        target.session_id
                    )
                })?;
                set_session_batch_result(
                    &mut results,
                    target.index,
                    session_batch_result(
                        target.session_id,
                        "timeout",
                        None,
                        None,
                        Some(wait_payload(
                            snapshot,
                            "timeout",
                            after_id,
                            timeout_ms,
                            if after_id.is_some() {
                                target.observed_events
                            } else {
                                Vec::new()
                            },
                            target.next_after_id,
                        )),
                    ),
                )?;
            }

            let results = collect_session_batch_results(results)?;
            return Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: session_wait_batch_payload(
                    current_session_id,
                    after_id,
                    timeout_ms,
                    results,
                ),
            });
        }

        let remaining_ms = timeout_ms - elapsed_ms;
        let drained: Vec<InterAgentMessage> = mailbox.drain().await;
        if !drained.is_empty() {
            continue;
        }

        let wait_result = timeout(
            Duration::from_millis(remaining_ms),
            mailbox_subscription.changed(),
        )
        .await;
        if let Ok(Err(_)) = wait_result {
            return Err("session_wait_internal_error: mailbox subscription closed".to_owned());
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn observe_visible_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    recent_event_limit: usize,
    tail_after_id: Option<i64>,
    tail_page_limit: usize,
) -> Result<SessionObservationSnapshot, String> {
    let target_session_id = normalize_required_session_id(target_session_id)?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let (observation, delegate_events, tree) = repo.with_read_snapshot(|conn| {
        let observation = SessionRepository::load_session_observation_with_conn(
            conn,
            &target_session_id,
            recent_event_limit,
            tail_after_id,
            tail_page_limit,
        )?
        .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        let delegate_events = if observation.session.kind == SessionKind::DelegateChild {
            SessionRepository::list_delegate_lifecycle_events_with_conn(
                conn,
                &observation.session.session_id,
            )?
        } else {
            Vec::new()
        };
        let tree =
            load_session_tree_snapshot_record_with_conn(conn, &observation.session.session_id)?;
        Ok((observation, delegate_events, tree))
    })?;
    let SessionObservationRecord {
        session,
        terminal_outcome,
        recent_events,
        tail_events,
    } = observation;
    let workflow = load_session_workflow_record(&repo, &session, Some(delegate_events.as_slice()))?;
    let delegate_lifecycle =
        session_delegate_lifecycle_at(&session, delegate_events.as_slice(), current_unix_ts());
    let subagent_contract = resolve_subagent_contract_for_session(
        &repo,
        &session,
        delegate_lifecycle.as_ref(),
        tool_config,
    )?;

    Ok(SessionObservationSnapshot {
        inspection: SessionInspectionSnapshot {
            session,
            terminal_outcome,
            recent_events,
            delegate_events,
            workflow,
            tree,
            subagent_contract,
        },
        tail_events,
    })
}

#[cfg(feature = "memory-sqlite")]
fn load_session_tree_snapshot_record_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<SessionTreeSnapshotRecord, String> {
    let heads = SessionRepository::list_session_heads_with_conn(conn, session_id)?;
    let active_path = SessionRepository::load_session_path_for_head_with_conn(
        conn,
        session_id,
        crate::session::repository::ACTIVE_SESSION_HEAD_NAME,
    )?;
    let artifacts = SessionRepository::list_session_artifacts_with_conn(conn, session_id)?;

    Ok(SessionTreeSnapshotRecord {
        heads,
        active_path,
        artifacts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_subagent_contract_for_session(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
    tool_config: &ToolConfig,
) -> Result<Option<ConstrainedSubagentContractView>, String> {
    if session.kind != SessionKind::DelegateChild {
        return Ok(None);
    }

    if let Some(contract) =
        delegate_lifecycle.and_then(resolve_subagent_contract_from_delegate_lifecycle)
    {
        return Ok(Some(attach_session_label_identity(contract, session)));
    }

    if session.parent_session_id.is_none() {
        return Ok(None);
    }

    let depth = match repo.session_lineage_depth(&session.session_id) {
        Ok(depth) => depth,
        Err(error) if is_expected_lineage_gap_error(&error) => {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "compute session lineage depth for subagent profile failed: {error}"
            ));
        }
    };

    Ok(Some(attach_session_label_identity(
        ConstrainedSubagentContractView::from_profile(ConstrainedSubagentProfile::for_child_depth(
            depth,
            tool_config.delegate.max_depth,
        )),
        session,
    )))
}

#[cfg(feature = "memory-sqlite")]
fn attach_session_label_identity(
    mut contract: ConstrainedSubagentContractView,
    session: &SessionSummaryRecord,
) -> ConstrainedSubagentContractView {
    if contract.identity.is_none() {
        let nickname = session.label.clone();
        if nickname.is_some() {
            contract = contract.with_identity(ConstrainedSubagentIdentity {
                nickname,
                specialization: None,
            });
        }
    }
    contract
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_lifecycle_events(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
) -> Result<Vec<SessionEventRecord>, String> {
    if session.kind != SessionKind::DelegateChild {
        return Ok(Vec::new());
    }
    repo.list_delegate_lifecycle_events(&session.session_id)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_session_workflow_record(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_events: Option<&[SessionEventRecord]>,
) -> Result<SessionWorkflowRecord, String> {
    let lineage_root_session_id =
        optional_lineage_lookup(repo.lineage_root_session_id(&session.session_id))?.flatten();
    let lineage_depth = optional_lineage_lookup(repo.session_lineage_depth(&session.session_id))?;

    let loaded_delegate_events = match delegate_events {
        Some(_) => None,
        None if session.kind == SessionKind::DelegateChild => {
            Some(repo.list_delegate_lifecycle_events(&session.session_id)?)
        }
        None => None,
    };
    let delegate_events = match delegate_events {
        Some(events) => events,
        None => loaded_delegate_events.as_deref().unwrap_or(&[]),
    };
    let workflow_id = session_workflow_id(session, lineage_root_session_id.as_deref());
    let task = delegate_events
        .iter()
        .rev()
        .find_map(session_workflow_task_from_event);
    let phase = session_workflow_phase(session, delegate_events);
    let operation_kind = session_workflow_operation_kind(session);
    let operation_scope = session_workflow_operation_scope(session);
    let task_session_id = session_workflow_task_session_id(session);
    let task_progress = repo
        .load_latest_event_by_kind(&session.session_id, TASK_PROGRESS_EVENT_KIND)?
        .as_ref()
        .and_then(|event| task_progress_from_event_payload(&event.payload_json));
    let resolved_task_identity = if session.kind == SessionKind::DelegateChild {
        Some(resolve_task_identity_for_session(repo, &session.session_id))
    } else {
        None
    };
    let runtime_self_continuity =
        load_session_runtime_self_continuity_record(repo, session, delegate_events)?;
    let binding =
        session_workflow_binding_record(session, delegate_events, resolved_task_identity.as_ref());

    Ok(SessionWorkflowRecord {
        workflow_id,
        task,
        phase,
        operation_kind,
        operation_scope,
        task_session_id,
        lineage_root_session_id,
        lineage_depth,
        task_progress,
        runtime_self_continuity,
        binding,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_id(
    session: &SessionSummaryRecord,
    lineage_root_session_id: Option<&str>,
) -> String {
    let fallback_session_id = session.session_id.as_str();
    let resolved_workflow_id = lineage_root_session_id.unwrap_or(fallback_session_id);
    resolved_workflow_id.to_owned()
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_task_from_event(event: &SessionEventRecord) -> Option<String> {
    event
        .payload_json
        .get("task")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_phase(
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
) -> Option<GovernedWorkflowPhase> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let cancellation_reason = session
        .last_error
        .as_deref()
        .and_then(parse_delegate_cancelled_reason);
    let was_cancelled = cancellation_reason.is_some();
    if was_cancelled {
        return Some(GovernedWorkflowPhase::Cancelled);
    }

    let has_delegate_events = !delegate_events.is_empty();
    if !has_delegate_events && session.parent_session_id.is_none() {
        return None;
    }

    match session.state {
        SessionState::Ready => Some(GovernedWorkflowPhase::Execute),
        SessionState::Running => Some(GovernedWorkflowPhase::Execute),
        SessionState::Completed => Some(GovernedWorkflowPhase::Complete),
        SessionState::Failed => Some(GovernedWorkflowPhase::Failed),
        SessionState::TimedOut => Some(GovernedWorkflowPhase::Failed),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_operation_kind(
    session: &SessionSummaryRecord,
) -> Option<WorkflowOperationKind> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(WorkflowOperationKind::Task)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_operation_scope(
    session: &SessionSummaryRecord,
) -> Option<WorkflowOperationScope> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(WorkflowOperationScope::Task)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_task_session_id(session: &SessionSummaryRecord) -> Option<String> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(session.session_id.clone())
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_record(
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
    resolved_task_identity: Option<&crate::task_progress::ResolvedTaskIdentity>,
) -> Option<SessionWorkflowBindingRecord> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let (execution, execution_surface) =
        latest_delegate_execution_binding_components(delegate_events)?;
    let mode = session_workflow_binding_mode(&execution);
    let worktree = session_workflow_worktree_binding(session, &execution);
    let task_id = resolved_task_identity
        .map(|task_identity| task_identity.task_id.clone())
        .unwrap_or_else(|| session.session_id.clone());
    let task_session_id = resolved_task_identity
        .map(|task_identity| task_identity.task_session_id.clone())
        .unwrap_or_else(|| session.session_id.clone());
    let binding = SessionWorkflowBindingRecord {
        session_id: session.session_id.clone(),
        task_id,
        task_session_id,
        mode,
        execution_surface,
        worktree,
    };

    Some(binding)
}

#[cfg(feature = "memory-sqlite")]
fn latest_delegate_execution_binding_components(
    delegate_events: &[SessionEventRecord],
) -> Option<(ConstrainedSubagentExecution, String)> {
    for event in delegate_events.iter().rev() {
        let event_kind = event.event_kind.as_str();
        let is_delegate_execution_event =
            matches!(event_kind, "delegate_queued" | "delegate_started");
        if !is_delegate_execution_event {
            continue;
        }

        let execution = ConstrainedSubagentExecution::from_event_payload(&event.payload_json)?;
        let execution_surface = session_workflow_execution_surface(event, &execution);
        return Some((execution, execution_surface));
    }

    None
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_execution_surface(
    event: &SessionEventRecord,
    execution: &ConstrainedSubagentExecution,
) -> String {
    let trust_event = crate::trust::extract_trust_event_payload(&event.payload_json);
    if let Some(trust_event) = trust_event {
        return trust_event.source_surface;
    }

    let fallback_surface = match execution.mode {
        crate::conversation::ConstrainedSubagentMode::Inline => "delegate.inline",
        crate::conversation::ConstrainedSubagentMode::Async => "delegate.async",
    };

    fallback_surface.to_owned()
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_mode(execution: &ConstrainedSubagentExecution) -> GovernedSessionMode {
    if execution.kernel_bound {
        return GovernedSessionMode::MutatingCapable;
    }

    GovernedSessionMode::AdvisoryOnly
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_worktree_binding(
    session: &SessionSummaryRecord,
    execution: &ConstrainedSubagentExecution,
) -> Option<WorktreeBindingDescriptor> {
    let workspace_root = execution.workspace_root.as_ref()?;
    let workspace_root_string = workspace_root.display().to_string();
    let worktree = WorktreeBindingDescriptor {
        worktree_id: session.session_id.clone(),
        workspace_root: workspace_root_string,
    };

    Some(worktree)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn optional_lineage_lookup<T>(result: Result<T, String>) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_expected_lineage_gap_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn is_expected_lineage_gap_error(error: &str) -> bool {
    let is_broken = error.starts_with("session_lineage_broken:");
    let is_cycle = error.starts_with("session_lineage_cycle_detected:");
    is_broken || is_cycle
}

#[cfg(feature = "memory-sqlite")]
fn load_session_runtime_self_continuity_record(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
) -> Result<Option<SessionRuntimeSelfContinuityRecord>, String> {
    let continuity =
        runtime_self_continuity::load_persisted_runtime_self_continuity_with_delegate_events(
            repo,
            &session.session_id,
            Some(delegate_events),
        )?;
    let record = continuity
        .as_ref()
        .map(session_runtime_self_continuity_record_from_continuity);
    Ok(record)
}

#[cfg(feature = "memory-sqlite")]
fn session_runtime_self_continuity_record_from_continuity(
    continuity: &runtime_self_continuity::RuntimeSelfContinuity,
) -> SessionRuntimeSelfContinuityRecord {
    let session_profile_projection_present = continuity
        .session_profile_projection
        .as_deref()
        .is_some_and(|projection| !projection.trim().is_empty());
    SessionRuntimeSelfContinuityRecord {
        present: continuity.has_prompt_projection(),
        resolved_identity_present: continuity.resolved_identity.is_some(),
        session_profile_projection_present,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_state_is_terminal(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Completed | SessionState::Failed | SessionState::TimedOut
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_inspection_payload(snapshot: SessionInspectionSnapshot) -> Value {
    let terminal_outcome_state =
        session_terminal_outcome_state(snapshot.session.state, snapshot.terminal_outcome.is_some());
    let delegate_lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.delegate_events.as_slice(),
        current_unix_ts(),
    );
    let recovery = match terminal_outcome_state {
        "missing" => Some(observe_missing_recovery(
            snapshot.recent_events.as_slice(),
            snapshot.session.last_error.as_deref(),
        )),
        _ => None,
    };
    let terminal_outcome_missing_reason = match terminal_outcome_state {
        "missing" => session_terminal_outcome_missing_reason(recovery.as_ref()),
        _ => None,
    };
    let diagnostics =
        session_diagnostics_json(&snapshot, terminal_outcome_state, recovery.as_ref());
    let subagent_handle = subagent_handle_for_session(
        &snapshot.session,
        snapshot.subagent_contract.as_ref(),
        delegate_lifecycle.as_ref(),
    );
    let tree = session_tree_snapshot_json(&snapshot.tree);
    let mut payload = json!({
        "session": {
            "session_id": snapshot.session.session_id,
            "kind": snapshot.session.kind.as_str(),
            "parent_session_id": snapshot.session.parent_session_id,
            "label": snapshot.session.label,
            "state": snapshot.session.state.as_str(),
            "created_at": snapshot.session.created_at,
            "updated_at": snapshot.session.updated_at,
            "archived": snapshot.session.archived_at.is_some(),
            "archived_at": snapshot.session.archived_at,
            "turn_count": snapshot.session.turn_count,
            "last_turn_at": snapshot.session.last_turn_at,
            "last_error": snapshot.session.last_error,
        },
        "task_progress": snapshot.workflow.task_progress,
        "workflow": session_workflow_json(snapshot.workflow),
        "tree": tree,
        "terminal_outcome_state": terminal_outcome_state,
        "terminal_outcome_missing_reason": terminal_outcome_missing_reason,
        "diagnostics": diagnostics,
        "delegate_lifecycle": delegate_lifecycle
            .map(|lifecycle| session_delegate_lifecycle_json(
                lifecycle,
                snapshot.subagent_contract.as_ref(),
            )),
        "recovery": recovery.map(recovery_json),
        "terminal_outcome": snapshot.terminal_outcome.map(session_terminal_outcome_json),
        "recent_events": snapshot
            .recent_events
            .into_iter()
            .map(session_event_json)
            .collect::<Vec<_>>(),
    });
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    insert_subagent_surface_fields(
        object,
        snapshot.subagent_contract.as_ref(),
        subagent_handle.as_ref(),
    );
    payload
}

#[cfg(feature = "memory-sqlite")]
fn session_tree_snapshot_json(snapshot: &SessionTreeSnapshotRecord) -> Value {
    let active_head_name = snapshot
        .heads
        .iter()
        .find(|head| head.head_name == "active")
        .map(|head| head.head_name.clone());
    let checkpoint_count = snapshot
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == SessionArtifactKind::Checkpoint)
        .count();
    let branch_summary_count = snapshot
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == SessionArtifactKind::BranchSummary)
        .count();
    let compaction_summary_count = snapshot
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == SessionArtifactKind::CompactionSummary)
        .count();
    let handoff_count = snapshot
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == SessionArtifactKind::Handoff)
        .count();
    let note_count = snapshot
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == SessionArtifactKind::Note)
        .count();

    json!({
        "head_count": snapshot.heads.len(),
        "active_path_count": snapshot.active_path.len(),
        "artifact_count": snapshot.artifacts.len(),
        "active_head_name": active_head_name,
        "artifact_counts": {
            "checkpoint": checkpoint_count,
            "branch_summary": branch_summary_count,
            "compaction_summary": compaction_summary_count,
            "handoff": handoff_count,
            "note": note_count,
        },
        "heads": snapshot.heads.iter().map(session_head_payload).collect::<Vec<_>>(),
        "active_path": snapshot.active_path.iter().map(session_node_payload).collect::<Vec<_>>(),
        "artifacts": snapshot.artifacts.iter().map(session_artifact_payload).collect::<Vec<_>>(),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_diagnostics_json(
    snapshot: &SessionInspectionSnapshot,
    terminal_outcome_state: &str,
    recovery: Option<&SessionRecoveryRecord>,
) -> Value {
    let recent_events = snapshot.recent_events.as_slice();
    let latest_provider_failover = latest_provider_failover_diagnostic(recent_events);
    let recommended_action = recommended_session_action(snapshot);
    let attention_hints = build_session_attention_hints(
        latest_provider_failover.as_ref(),
        recommended_action.as_ref(),
        recovery,
        terminal_outcome_state,
    );

    json!({
        "latest_provider_failover": latest_provider_failover,
        "recommended_action": recommended_action,
        "attention_hints": attention_hints,
    })
}

#[cfg(feature = "memory-sqlite")]
fn latest_provider_failover_diagnostic(recent_events: &[SessionEventRecord]) -> Option<Value> {
    let matching_event = recent_events
        .iter()
        .filter(|event| event.event_kind == "trust_provider_failover")
        .max_by_key(|event| event.ts)?;
    let payload_object = matching_event.payload_json.as_object()?;
    let provider_failover = payload_object.get("provider_failover")?.as_object()?;

    let provider_id = payload_object
        .get("provider_id")
        .cloned()
        .unwrap_or(Value::Null);
    let binding = payload_object
        .get("binding")
        .cloned()
        .unwrap_or(Value::Null);
    let reason = provider_failover
        .get("reason")
        .cloned()
        .unwrap_or(Value::Null);
    let stage = provider_failover
        .get("stage")
        .cloned()
        .unwrap_or(Value::Null);
    let model = provider_failover
        .get("model")
        .cloned()
        .unwrap_or(Value::Null);
    let attempt = provider_failover
        .get("attempt")
        .cloned()
        .unwrap_or(Value::Null);
    let max_attempts = provider_failover
        .get("max_attempts")
        .cloned()
        .unwrap_or(Value::Null);
    let status_code = provider_failover
        .get("status_code")
        .cloned()
        .unwrap_or(Value::Null);
    let request_id = provider_failover
        .get("request_id")
        .cloned()
        .unwrap_or(Value::Null);
    let cf_ray = provider_failover
        .get("cf_ray")
        .cloned()
        .unwrap_or(Value::Null);
    let auth_error = provider_failover
        .get("auth_error")
        .cloned()
        .unwrap_or(Value::Null);
    let auth_error_code = provider_failover
        .get("auth_error_code")
        .cloned()
        .unwrap_or(Value::Null);

    Some(json!({
        "event_id": matching_event.id,
        "event_kind": matching_event.event_kind,
        "ts": matching_event.ts,
        "provider_id": provider_id,
        "binding": binding,
        "reason": reason,
        "stage": stage,
        "model": model,
        "attempt": attempt,
        "max_attempts": max_attempts,
        "status_code": status_code,
        "request_id": request_id,
        "cf_ray": cf_ray,
        "auth_error": auth_error,
        "auth_error_code": auth_error_code,
    }))
}

#[cfg(feature = "memory-sqlite")]
fn recommended_session_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let recover_action = recommended_recover_action(snapshot);
    if recover_action.is_some() {
        return recover_action;
    }

    recommended_resume_action(snapshot)
}

#[cfg(feature = "memory-sqlite")]
fn recommended_recover_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let recover_plan = build_session_recover_plan(snapshot, current_unix_ts()).ok()?;
    let mut recover_action = session_recovery_action_json(&recover_plan);
    let action_object = recover_action.as_object_mut()?;
    action_object.insert(
        "source".to_owned(),
        Value::String("session_recover_plan".to_owned()),
    );
    action_object.insert(
        "tool_name".to_owned(),
        Value::String("session_recover".to_owned()),
    );
    action_object.insert("requires_mutation".to_owned(), Value::Bool(true));
    Some(recover_action)
}

#[cfg(feature = "memory-sqlite")]
fn recommended_resume_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let task_progress = snapshot.workflow.task_progress.as_ref()?;
    let resume_recipe = task_progress.resume_recipe.as_ref()?;
    let tool_name = resume_recipe.recommended_tool.clone();
    let session_id = resume_recipe.task_session_id.clone();
    let note = resume_recipe.note.clone();
    let requires_mutation = session_action_requires_mutation(tool_name.as_str());
    let task_status = task_progress.status.as_str().to_owned();

    Some(json!({
        "source": "task_progress_resume_recipe",
        "kind": "follow_resume_recipe",
        "tool_name": tool_name,
        "session_id": session_id,
        "note": note,
        "task_status": task_status,
        "requires_mutation": requires_mutation,
    }))
}

#[cfg(feature = "memory-sqlite")]
fn session_action_requires_mutation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "session_archive" | "session_cancel" | "session_continue" | "session_recover"
    )
}

#[cfg(feature = "memory-sqlite")]
fn build_session_attention_hints(
    latest_provider_failover: Option<&Value>,
    recommended_action: Option<&Value>,
    recovery: Option<&SessionRecoveryRecord>,
    terminal_outcome_state: &str,
) -> Vec<String> {
    let mut hints = Vec::new();

    if let Some(provider_failover) = latest_provider_failover {
        let reason = provider_failover
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let model = provider_failover
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let stage = provider_failover
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let request_id = provider_failover
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let auth_error_code = provider_failover
            .get("auth_error_code")
            .and_then(Value::as_str)
            .unwrap_or("-");
        hints.push(format!(
            "provider_failover_present reason={reason} model={model} stage={stage} request_id={request_id} auth_error_code={auth_error_code}"
        ));
    }

    if let Some(action) = recommended_action {
        let tool_name = action
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let kind = action
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let source = action
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        hints.push(format!(
            "recommended_action tool={tool_name} kind={kind} source={source}"
        ));
    }

    if terminal_outcome_state == "missing" {
        let recovery_kind = recovery
            .map(|record| record.kind.as_str())
            .unwrap_or("unknown");
        let recovery_source = recovery
            .map(|record| record.source.as_str())
            .unwrap_or("none");
        hints.push(format!(
            "terminal_outcome_missing kind={recovery_kind} source={recovery_source}"
        ));
    }

    hints
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_state(state: SessionState, has_terminal_outcome: bool) -> &'static str {
    if has_terminal_outcome {
        "present"
    } else if session_state_is_terminal(state) {
        "missing"
    } else {
        "not_terminal"
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_missing_reason(
    recovery: Option<&SessionRecoveryRecord>,
) -> Option<String> {
    recovery.map(|recovery| recovery.kind.clone())
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_status_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        target_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        5,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };

    Ok(session_batch_result(
        target_session_id.to_owned(),
        "ok",
        None,
        None,
        Some(session_inspection_payload(snapshot)),
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_batch_payload(
    tool: &str,
    current_session_id: &str,
    dry_run: bool,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    session_batch_payload_with_optional_dry_run(
        tool,
        current_session_id,
        requested_count,
        results,
        Some(dry_run),
    )
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_payload_without_dry_run(
    tool: &str,
    current_session_id: &str,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    session_batch_payload_with_optional_dry_run(
        tool,
        current_session_id,
        requested_count,
        results,
        None,
    )
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_payload_with_optional_dry_run(
    tool: &str,
    current_session_id: &str,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
    dry_run: Option<bool>,
) -> Value {
    let mut result_counts = BTreeMap::<&'static str, usize>::new();
    for result in &results {
        *result_counts.entry(result.result).or_default() += 1;
    }

    let mut payload = json!({
        "tool": tool,
        "current_session_id": current_session_id,
        "requested_count": requested_count,
        "result_counts": result_counts,
        "results": results.into_iter().map(session_batch_result_json).collect::<Vec<_>>(),
    });
    if let Some(dry_run) = dry_run
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("dry_run".to_owned(), Value::Bool(dry_run));
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn session_wait_batch_payload(
    current_session_id: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    let mut payload = session_batch_payload_without_dry_run(
        "session_wait",
        current_session_id,
        results.len(),
        results,
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("timeout_ms".to_owned(), Value::from(timeout_ms));
        object.insert(
            "after_id".to_owned(),
            after_id.map(Value::from).unwrap_or(Value::Null),
        );
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_batch_result(
    session_id: String,
    result: &'static str,
    message: Option<String>,
    action: Option<Value>,
    inspection: Option<Value>,
) -> SessionBatchResultRecord {
    SessionBatchResultRecord {
        session_id,
        result,
        message,
        action,
        inspection,
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn wait_outcome(
    status: &str,
    snapshot: SessionInspectionSnapshot,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: status.to_owned(),
        payload: wait_payload(
            snapshot,
            if status == "ok" {
                "completed"
            } else {
                "timeout"
            },
            after_id,
            timeout_ms,
            observed_events,
            next_after_id,
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn wait_payload(
    snapshot: SessionInspectionSnapshot,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> Value {
    let next_after_id = match after_id {
        Some(_) => next_after_id,
        None => snapshot
            .recent_events
            .last()
            .map(|event| event.id)
            .unwrap_or(0),
    };
    let events = match after_id {
        Some(_) => observed_events,
        None => snapshot.recent_events.clone(),
    };
    let mut payload = session_inspection_payload(snapshot);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "wait_status".to_owned(),
            Value::String(wait_status.to_owned()),
        );
        if wait_status != "completed" {
            let continuation_note =
                continuation_note_for_wait_status(wait_status, object.get("session"));
            let session_id = object
                .get("session")
                .and_then(|value| value.get("session_id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let recommended_payload = json!({
                "session_id": session_id,
                "timeout_ms": timeout_ms,
            });
            object.insert(
                "continuation".to_owned(),
                json!({
                    "state": wait_status,
                    "is_terminal": false,
                    "recommended_tool": "session_wait",
                    "recommended_payload": recommended_payload,
                    "note": continuation_note,
                }),
            );
        }
        object.insert("timeout_ms".to_owned(), Value::from(timeout_ms));
        object.insert(
            "after_id".to_owned(),
            after_id.map(Value::from).unwrap_or(Value::Null),
        );
        object.insert("next_after_id".to_owned(), Value::from(next_after_id));
        object.insert(
            "events".to_owned(),
            Value::Array(
                events
                    .into_iter()
                    .map(session_event_json)
                    .collect::<Vec<_>>(),
            ),
        );
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
fn continuation_note_for_wait_status(wait_status: &str, session_payload: Option<&Value>) -> String {
    let session_state = session_payload
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match wait_status {
        "waiting" => format!(
            "The runtime is still waiting on session state `{session_state}`. Treat this as intermediate progress, not final completion."
        ),
        "blocked" => format!(
            "The runtime is blocked while the session state is `{session_state}`. Report the exact blocker or resolve it before presenting final completion."
        ),
        other => format!(
            "The runtime is still in non-terminal wait state `{other}` with session state `{session_state}`."
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn task_wait_outcome(
    status: &str,
    snapshot: SessionInspectionSnapshot,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: status.to_owned(),
        payload: task_wait_payload(
            snapshot,
            lineage_records,
            current_owner_session_id,
            wait_status,
            after_id,
            timeout_ms,
            observed_events,
            next_after_id,
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn task_wait_payload(
    snapshot: SessionInspectionSnapshot,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> Value {
    let payload = wait_payload(
        snapshot,
        wait_status,
        after_id,
        timeout_ms,
        observed_events,
        next_after_id,
    );
    let task_events = payload
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    event.get("event_kind").and_then(Value::as_str)
                        == Some(TASK_PROGRESS_EVENT_KIND)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut payload = rewrite_task_payload_aliases(payload, "task_wait");
    let task_state = task_state_from_payload(&payload);
    if let Some(object) = payload.as_object_mut() {
        object.insert("task_events".to_owned(), Value::Array(task_events));
        if wait_status != "completed" {
            let task_id = object
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let recommended_payload = json!({
                "task_id": task_id,
                "timeout_ms": timeout_ms,
            });
            let continuation_value = object
                .entry("continuation".to_owned())
                .or_insert_with(|| json!({}));
            if let Some(continuation_object) = continuation_value.as_object_mut() {
                continuation_object.insert(
                    "recommended_tool".to_owned(),
                    Value::String("task_wait".to_owned()),
                );
                continuation_object.insert("recommended_payload".to_owned(), recommended_payload);
            }
        }
    }
    let payload = decorate_task_status_payload(payload, task_state);
    decorate_task_lineage_payload(payload, lineage_records, current_owner_session_id)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_batch_result_json(result: SessionBatchResultRecord) -> Value {
    json!({
        "session_id": result.session_id,
        "result": result.result,
        "message": result.message,
        "action": result.action,
        "inspection": result.inspection,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn is_session_visibility_skip_error(error: &str) -> bool {
    error.starts_with("visibility_denied:") || error.starts_with("session_not_found:")
}

#[cfg(feature = "memory-sqlite")]
fn sessions_list_filters_json(request: &SessionsListRequest) -> Value {
    json!({
        "limit": request.limit,
        "offset": request.offset,
        "state": request.state.map(SessionState::as_str),
        "kind": request.kind.map(SessionKind::as_str),
        "parent_session_id": request.parent_session_id.clone(),
        "overdue_only": request.overdue_only,
        "include_archived": request.include_archived,
        "include_delegate_lifecycle": request.effective_include_delegate_lifecycle(),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json(session: SessionSummaryRecord, workflow: SessionWorkflowRecord) -> Value {
    json!({
        "session_id": session.session_id,
        "kind": session.kind.as_str(),
        "parent_session_id": session.parent_session_id,
        "label": session.label,
        "state": session.state.as_str(),
        "created_at": session.created_at,
        "updated_at": session.updated_at,
        "archived": session.archived_at.is_some(),
        "archived_at": session.archived_at,
        "turn_count": session.turn_count,
        "last_turn_at": session.last_turn_at,
        "last_error": session.last_error,
        "workflow": session_workflow_json(workflow),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json_with_delegate_lifecycle(
    session: SessionSummaryRecord,
    workflow: SessionWorkflowRecord,
    delegate_lifecycle: Option<SessionDelegateLifecycleRecord>,
    subagent_contract: Option<ConstrainedSubagentContractView>,
    include_delegate_lifecycle: bool,
) -> Value {
    let subagent = subagent_handle_for_session(
        &session,
        subagent_contract.as_ref(),
        delegate_lifecycle.as_ref(),
    );
    let mut payload = session_summary_json(session, workflow);
    if let Some(object) = payload.as_object_mut() {
        insert_subagent_surface_fields(object, subagent_contract.as_ref(), subagent.as_ref());
        if include_delegate_lifecycle {
            object.insert(
                "delegate_lifecycle".to_owned(),
                delegate_lifecycle
                    .map(|lifecycle| {
                        session_delegate_lifecycle_json(lifecycle, subagent_contract.as_ref())
                    })
                    .unwrap_or(Value::Null),
            );
        }
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
fn insert_subagent_surface_fields(
    object: &mut serde_json::Map<String, Value>,
    _subagent_contract: Option<&ConstrainedSubagentContractView>,
    subagent: Option<&ConstrainedSubagentHandle>,
) {
    object.extend(subagent_surface_fields(subagent));
}

#[cfg(feature = "memory-sqlite")]
fn subagent_handle_for_session(
    session: &SessionSummaryRecord,
    subagent_contract: Option<&ConstrainedSubagentContractView>,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
) -> Option<ConstrainedSubagentHandle> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let phase = delegate_lifecycle.map(|lifecycle| lifecycle.phase.to_owned());
    Some(
        ConstrainedSubagentHandle::new(session.session_id.clone())
            .with_parent_session_id(session.parent_session_id.clone())
            .with_label(session.label.clone())
            .with_state(Some(session.state.as_str().to_owned()))
            .with_phase(phase.clone())
            .with_identity(
                subagent_contract
                    .and_then(ConstrainedSubagentContractView::resolved_identity)
                    .cloned(),
            )
            .with_contract(subagent_contract.cloned())
            .with_coordination(subagent_handle_coordination_actions(
                session,
                phase.as_deref(),
                delegate_lifecycle,
                subagent_contract,
            )),
    )
}

#[cfg(feature = "memory-sqlite")]
fn subagent_handle_coordination_actions(
    session: &SessionSummaryRecord,
    phase: Option<&str>,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
    subagent_contract: Option<&ConstrainedSubagentContractView>,
) -> Vec<crate::conversation::ConstrainedSubagentCoordinationAction> {
    let is_async = delegate_lifecycle
        .map(|lifecycle| lifecycle.mode == "async")
        .unwrap_or_else(|| {
            matches!(
                subagent_contract.and_then(|contract| contract.mode),
                Some(crate::conversation::ConstrainedSubagentMode::Async)
            )
        });
    let overdue = matches!(
        delegate_lifecycle.and_then(|lifecycle| lifecycle.staleness.as_ref()),
        Some(staleness) if staleness.state == "overdue"
    );
    let mode = if is_async {
        Some(crate::conversation::ConstrainedSubagentMode::Async)
    } else {
        subagent_contract.and_then(|contract| contract.mode)
    };
    let terminal_but_not_archived =
        session_state_is_terminal(session.state) && session.archived_at.is_none();
    coordination_actions_for_subagent_handle(terminal_but_not_archived, phase, mode, overdue)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_json(workflow: SessionWorkflowRecord) -> Value {
    json!({
        "workflow_id": workflow.workflow_id,
        "task": workflow.task,
        "phase": workflow.phase,
        "operation_kind": workflow.operation_kind,
        "operation_scope": workflow.operation_scope,
        "task_session_id": workflow.task_session_id,
        "lineage_root_session_id": workflow.lineage_root_session_id,
        "lineage_depth": workflow.lineage_depth,
        "task_progress": workflow.task_progress,
        "runtime_self_continuity": workflow
            .runtime_self_continuity
            .map(session_runtime_self_continuity_json),
        "binding": workflow.binding.map(session_workflow_binding_json),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_runtime_self_continuity_json(
    runtime_self_continuity: SessionRuntimeSelfContinuityRecord,
) -> Value {
    json!({
        "present": runtime_self_continuity.present,
        "resolved_identity_present": runtime_self_continuity.resolved_identity_present,
        "session_profile_projection_present": runtime_self_continuity
            .session_profile_projection_present,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_json(binding: SessionWorkflowBindingRecord) -> Value {
    json!({
        "session_id": binding.session_id,
        "task_id": binding.task_id,
        "task_session_id": binding.task_session_id,
        "mode": binding.mode,
        "execution_surface": binding.execution_surface,
        "worktree": binding.worktree,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_event_json(event: SessionEventRecord) -> Value {
    json!({
        "id": event.id,
        "session_id": event.session_id,
        "event_kind": event.event_kind,
        "actor_session_id": event.actor_session_id,
        "payload_json": event.payload_json,
        "ts": event.ts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_json(
    outcome: crate::session::repository::SessionTerminalOutcomeRecord,
) -> Value {
    json!({
        "session_id": outcome.session_id,
        "status": outcome.status,
        "payload": outcome.payload_json,
        "frozen_result": outcome.frozen_result,
        "recorded_at": outcome.recorded_at,
    })
}
