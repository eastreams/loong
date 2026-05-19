use super::*;

pub(super) fn execute_session_tool_policy_status(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let target_session_id =
        resolve_session_tool_policy_target_session_id(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let policy = build_session_tool_policy_status_payload(&repo, &target_session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_status",
            "current_session_id": current_session_id,
            "target_session_id": target_session_id,
            "policy": policy,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_tool_policy_set(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_session_tool_policy_set_request(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &request.session_id,
        tool_config.sessions.visibility,
    )?;
    ensure_policy_target_session_exists(&repo, &request.session_id, current_session_id)?;

    let existing_policy = repo.load_session_tool_policy(&request.session_id)?;
    let existing_tool_ids = existing_policy
        .as_ref()
        .map(|policy| policy.requested_tool_ids.clone())
        .unwrap_or_default();
    let existing_runtime_narrowing = existing_policy
        .as_ref()
        .map(|policy| policy.runtime_narrowing.clone())
        .unwrap_or_default();

    let next_tool_ids = match request.tool_ids {
        Some(tool_ids) => {
            resolve_session_tool_policy_tool_ids(&repo, &request.session_id, tool_config, tool_ids)?
        }
        None => existing_tool_ids,
    };
    let next_runtime_narrowing = request
        .runtime_narrowing
        .unwrap_or(existing_runtime_narrowing);
    let clears_policy = next_tool_ids.is_empty() && next_runtime_narrowing.is_empty();

    let action = if clears_policy {
        if existing_policy.is_some() {
            repo.delete_session_tool_policy(&request.session_id)?;
            "cleared"
        } else {
            "unchanged"
        }
    } else {
        let next_policy = NewSessionToolPolicyRecord {
            session_id: request.session_id.clone(),
            requested_tool_ids: next_tool_ids.clone(),
            runtime_narrowing: next_runtime_narrowing.clone(),
        };
        let unchanged = existing_policy
            .as_ref()
            .is_some_and(|policy| policy.requested_tool_ids == next_tool_ids)
            && existing_policy
                .as_ref()
                .is_some_and(|policy| policy.runtime_narrowing == next_runtime_narrowing);
        if unchanged {
            "unchanged"
        } else {
            repo.upsert_session_tool_policy(next_policy)?;
            if existing_policy.is_some() {
                "updated"
            } else {
                "created"
            }
        }
    };
    let policy = build_session_tool_policy_status_payload(&repo, &request.session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_set",
            "action": action,
            "current_session_id": current_session_id,
            "target_session_id": request.session_id,
            "policy": policy,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_tool_policy_clear(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let target_session_id =
        resolve_session_tool_policy_target_session_id(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;

    let cleared = repo.delete_session_tool_policy(&target_session_id)?;
    let policy = build_session_tool_policy_status_payload(&repo, &target_session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_clear",
            "action": if cleared { "cleared" } else { "unchanged" },
            "current_session_id": current_session_id,
            "target_session_id": target_session_id,
            "policy": policy,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_fork_head(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let source_node_id = required_payload_string(&payload, "node_id", "session tool")?;
    let head_name = required_payload_string(&payload, "head_name", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let head = repo.fork_session_head(&target_session_id, &source_node_id, &head_name)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_fork_head",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "head": session_head_payload(&head),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_pin_head(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_session_set_head_mode(
        payload,
        current_session_id,
        config,
        tool_config,
        SessionHeadMode::Pinned,
        "session_pin_head",
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_set_active_head(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let head_name = required_payload_string(&payload, "head_name", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let head = repo
        .load_session_head(&target_session_id, &head_name)?
        .ok_or_else(|| format!("session head `{head_name}` not found"))?;
    let active_head = repo.set_session_head(
        &target_session_id,
        crate::session::repository::ACTIVE_SESSION_HEAD_NAME,
        &head.node_id,
    )?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_set_active_head",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "requested_head_name": head_name,
            "active_head": session_head_payload(&active_head),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_unpin_head(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_session_set_head_mode(
        payload,
        current_session_id,
        config,
        tool_config,
        SessionHeadMode::Live,
        "session_unpin_head",
    )
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_set_head_mode(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    head_mode: SessionHeadMode,
    tool_name: &str,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let head_name = required_payload_string(&payload, "head_name", "session tool")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let head = repo.set_session_head_mode(&target_session_id, &head_name, head_mode)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": tool_name,
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "head": session_head_payload(&head),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_recover(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let recover_plan = build_session_recover_plan(&snapshot, current_unix_ts())?;
        let outcome = apply_session_recover_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            &recover_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("recovery_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_recover_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_recover",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_task_recover(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let result = execute_session_recover_batch_result(
        &resolved_target.owner_session_id,
        current_session_id,
        config,
        tool_config,
        dry_run,
    )?;
    let mut payload = session_batch_result_json(result);
    payload = rewrite_task_payload_aliases(payload, "task_recover");
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_mutation_repository(config: &SessionStoreConfig) -> Result<SessionRepository, String> {
    SessionRepository::new(config).map(|repo| repo.with_max_total_artifacts(Some(1)))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_create_checkpoint(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let label = required_payload_string(&payload, "label", "session tool")?;
    let explicit_node_id = optional_payload_string(&payload, "node_id");
    let checkpoint_head_name = format!("checkpoint/{label}");
    let repo = session_mutation_repository(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;

    let anchor_node_id = if let Some(node_id) = explicit_node_id {
        let node = repo
            .load_session_node(&node_id)?
            .ok_or_else(|| format!("session node `{node_id}` not found"))?;
        if node.session_id != target_session_id {
            return Err(format!(
                "session node `{node_id}` belongs to `{}`, not `{target_session_id}`",
                node.session_id
            ));
        }
        node.node_id
    } else {
        let active_path = repo.load_active_session_path(&target_session_id)?;
        active_path
            .last()
            .map(|node| node.node_id.clone())
            .ok_or_else(|| format!("session `{target_session_id}` has no active path"))?
    };

    let checkpoint_ts = current_unix_ts();
    let artifact_id = format!(
        "checkpoint:{}:{}:{}",
        target_session_id,
        checkpoint_ts,
        label.replace('/', "_")
    );
    let checkpoint_head =
        repo.set_session_head(&target_session_id, &checkpoint_head_name, &anchor_node_id)?;
    let head = repo.set_session_head_mode(
        &target_session_id,
        &checkpoint_head.head_name,
        SessionHeadMode::Pinned,
    )?;
    let artifact = repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id,
        session_id: target_session_id.clone(),
        kind: SessionArtifactKind::Checkpoint,
        head_name: Some(checkpoint_head_name),
        anchor_node_id: Some(anchor_node_id.clone()),
        source_start_node_id: Some(anchor_node_id.clone()),
        source_end_node_id: Some(anchor_node_id),
        payload_json: json!({ "label": label }),
        summary_text: Some(label.clone()),
    })?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_create_checkpoint",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "label": label,
            "artifact": session_artifact_payload(&artifact),
            "head": session_head_payload(&head),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_create_branch_summary(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
    let head_name = required_payload_string(&payload, "head_name", "session tool")?;
    let summary_text = required_payload_string(&payload, "summary_text", "session tool")?;
    let explicit_anchor_node_id = optional_payload_string(&payload, "anchor_node_id");
    let repo = session_mutation_repository(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;

    let target_path = repo.load_session_path_for_head(&target_session_id, &head_name)?;
    if target_path.is_empty() {
        return Err(format!("session head `{head_name}` not found"));
    }
    let (anchor_node_id, source_start_node_id, source_end_node_id, metadata_json) =
        resolve_branch_summary_source_range(
            &repo,
            &target_session_id,
            &head_name,
            &target_path,
            explicit_anchor_node_id.as_deref(),
        )?;

    let artifact_id = format!(
        "branch-summary:{}:{}:{}",
        target_session_id,
        current_unix_ts(),
        head_name.replace('/', "_")
    );
    let artifact = repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id,
        session_id: target_session_id.clone(),
        kind: SessionArtifactKind::BranchSummary,
        head_name: Some(head_name.clone()),
        anchor_node_id: Some(anchor_node_id),
        source_start_node_id: Some(source_start_node_id),
        source_end_node_id: Some(source_end_node_id),
        payload_json: metadata_json,
        summary_text: Some(summary_text.clone()),
    })?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_create_branch_summary",
            "current_session_id": current_session_id,
            "session_id": target_session_id,
            "head_name": head_name,
            "summary_text": summary_text,
            "artifact": session_artifact_payload(&artifact),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_branch_summary_source_range(
    repo: &SessionRepository,
    session_id: &str,
    head_name: &str,
    target_path: &[SessionNodeRecord],
    explicit_anchor_node_id: Option<&str>,
) -> Result<(String, String, String, Value), String> {
    if let Some(anchor_node_id) = explicit_anchor_node_id {
        let anchor_index = target_path
            .iter()
            .position(|node| node.node_id == anchor_node_id)
            .ok_or_else(|| {
                format!("session node `{anchor_node_id}` is not on head `{head_name}` path")
            })?;
        let source_start = target_path.get(anchor_index + 1).ok_or_else(|| {
            format!(
                "branch summary anchor `{anchor_node_id}` does not have a descendant on head `{head_name}`"
            )
        })?;
        let source_end = target_path
            .last()
            .ok_or_else(|| format!("session head `{head_name}` has no tip node"))?;
        return Ok((
            anchor_node_id.to_owned(),
            source_start.node_id.clone(),
            source_end.node_id.clone(),
            json!({
                "head_name": head_name,
                "anchor_mode": "explicit",
                "exclusive_node_count": target_path.len().saturating_sub(anchor_index + 1),
                "session_id": session_id,
            }),
        ));
    }

    let active_path = repo.load_active_session_path(session_id)?;
    let common_prefix_len = target_path
        .iter()
        .zip(active_path.iter())
        .take_while(|(left, right)| left.node_id == right.node_id)
        .count();
    if common_prefix_len == 0 {
        return Err(format!(
            "head `{head_name}` does not share a common ancestor with the active path"
        ));
    }
    if common_prefix_len >= target_path.len() {
        return Err(format!(
            "head `{head_name}` has no exclusive branch segment relative to the active head"
        ));
    }

    let anchor_node = target_path
        .get(common_prefix_len - 1)
        .ok_or_else(|| format!("head `{head_name}` is missing a branch anchor"))?;
    let source_start = target_path
        .get(common_prefix_len)
        .ok_or_else(|| format!("head `{head_name}` is missing an exclusive branch start"))?;
    let source_end = target_path
        .last()
        .ok_or_else(|| format!("session head `{head_name}` has no tip node"))?;

    Ok((
        anchor_node.node_id.clone(),
        source_start.node_id.clone(),
        source_end.node_id.clone(),
        json!({
            "head_name": head_name,
            "anchor_mode": "implicit_active_path_fork",
            "exclusive_node_count": target_path.len().saturating_sub(common_prefix_len),
            "session_id": session_id,
        }),
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_cancel(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let cancel_plan = build_session_cancel_plan(&snapshot)?;
        let outcome = apply_session_cancel_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            cancel_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("cancel_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_cancel_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_cancel",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_task_cancel(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let result = execute_session_cancel_batch_result(
        &resolved_target.owner_session_id,
        current_session_id,
        config,
        tool_config,
        dry_run,
    )?;
    let mut payload = session_batch_result_json(result);
    payload = rewrite_task_payload_aliases(payload, "task_cancel");
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn execute_session_archive(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let archive_plan = build_session_archive_plan(&snapshot)?;
        let outcome = apply_session_archive_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            &archive_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("archive_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_archive_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_archive",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn build_session_archive_plan(
    snapshot: &SessionInspectionSnapshot,
) -> Result<SessionArchivePlan, String> {
    if snapshot.session.archived_at.is_some() {
        return Err(format!(
            "session_archive_not_archivable: session `{}` is already archived",
            snapshot.session.session_id
        ));
    }
    if !session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_archive_not_archivable: session `{}` is not terminal",
            snapshot.session.session_id
        ));
    }

    Ok(SessionArchivePlan {
        expected_state: snapshot.session.state,
    })
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_archive_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    archive_plan: &SessionArchivePlan,
) -> Result<SessionToolActionOutcome, String> {
    let transitioned = repo.transition_session_with_event_if_current(
        target_session_id,
        crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
            expected_state: archive_plan.expected_state,
            next_state: archive_plan.expected_state,
            last_error: snapshot.session.last_error.clone(),
            event_kind: "session_archived".to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            event_payload_json: json!({
                "previous_state": archive_plan.expected_state.as_str(),
                "hides_from_sessions_list": true,
            }),
        },
    )?;
    if transitioned.is_none() {
        let latest = repo
            .load_session_summary_with_legacy_fallback(target_session_id)?
            .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        return Err(format!(
            "session_archive_state_changed: session `{target_session_id}` is no longer archivable from state `{}`",
            latest.state.as_str()
        ));
    }

    let archived_snapshot = inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    Ok(SessionToolActionOutcome {
        inspection: session_inspection_payload(archived_snapshot),
        action: session_archive_action_json(archive_plan),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_archive_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
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
        10,
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
    let inspection = session_inspection_payload(snapshot.clone());
    let archive_plan = match build_session_archive_plan(&snapshot) {
        Ok(plan) => plan,
        Err(error)
            if error.starts_with("session_archive_not_archivable:")
                && error.contains("already archived") =>
        {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_already_archived",
                Some(error),
                None,
                Some(inspection),
            ));
        }
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_archivable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_archive_action_json(&archive_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_archive_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        &archive_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_archive_state_changed:") => {
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspect_visible_session_with_policies(
                    target_session_id,
                    current_session_id,
                    config,
                    tool_config,
                    10,
                )
                .ok()
                .map(session_inspection_payload),
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_archive_action_json(plan: &SessionArchivePlan) -> Value {
    json!({
        "kind": "session_archived",
        "previous_state": plan.expected_state.as_str(),
        "next_state": plan.expected_state.as_str(),
        "hides_from_sessions_list": true,
    })
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
fn execute_session_recover_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
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
        10,
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
    let inspection = session_inspection_payload(snapshot.clone());
    let recover_plan = match build_session_recover_plan(&snapshot, current_unix_ts()) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_recoverable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_recovery_action_json(&recover_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_recover_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        &recover_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_recover_state_changed:") => {
            let inspection = match inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            ) {
                Ok(snapshot) => Some(session_inspection_payload(snapshot)),
                Err(_) => None,
            };
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspection,
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_recover_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    recover_plan: &SessionRecoverPlan,
) -> Result<SessionToolActionOutcome, String> {
    let recovery_error = session_recovery_error(recover_plan);
    let outcome = delegate_error_outcome(
        snapshot.session.session_id.clone(),
        snapshot.session.label.clone(),
        recovery_error.clone(),
        recover_plan.elapsed_seconds.saturating_mul(1_000),
    );
    let frozen_result = capture_frozen_result(&outcome, tool_config.delegate.max_frozen_bytes);
    let outcome_status = outcome.status.clone();
    let outcome_payload = outcome.payload;
    let event_payload_json = match recover_plan.recovery_kind {
        RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED => {
            let Some(queued_at) = recover_plan.queued_at else {
                return Err(format!(
                    "session_recover_not_recoverable: session `{target_session_id}` is missing queued timestamp"
                ));
            };
            build_queued_async_overdue_recovery_payload(
                snapshot.session.label.as_deref(),
                queued_at,
                recover_plan.elapsed_seconds,
                recover_plan.timeout_seconds,
                recover_plan.deadline_at,
                &recovery_error,
            )
        }
        RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED => {
            build_running_async_overdue_recovery_payload(
                snapshot.session.label.as_deref(),
                recover_plan.queued_at,
                recover_plan.started_at,
                recover_plan.reference,
                recover_plan.elapsed_seconds,
                recover_plan.timeout_seconds,
                recover_plan.deadline_at,
                &recovery_error,
            )
        }
        other => {
            return Err(format!(
                "session_recover_not_supported: unsupported recovery kind `{other}`"
            ));
        }
    };
    let finalized = repo.finalize_session_terminal_if_current(
        target_session_id,
        recover_plan.expected_state,
        crate::session::repository::FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some(recovery_error),
            event_kind: RECOVERY_EVENT_KIND.to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            event_payload_json,
            outcome_status,
            outcome_payload_json: outcome_payload,
            frozen_result: Some(frozen_result),
        },
    )?;
    if finalized.is_none() {
        let latest = repo
            .load_session_summary_with_legacy_fallback(target_session_id)?
            .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        return Err(format!(
            "session_recover_state_changed: session `{target_session_id}` is no longer recoverable from state `{}`",
            latest.state.as_str()
        ));
    }
    let recovered_snapshot = inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    Ok(SessionToolActionOutcome {
        inspection: session_inspection_payload(recovered_snapshot),
        action: session_recovery_action_json(recover_plan),
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_recovery_action_json(plan: &SessionRecoverPlan) -> Value {
    json!({
        "kind": plan.recovery_kind,
        "previous_state": plan.expected_state.as_str(),
        "next_state": "failed",
        "reference": plan.reference,
        "elapsed_seconds": plan.elapsed_seconds,
        "timeout_seconds": plan.timeout_seconds,
        "deadline_at": plan.deadline_at,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_session_recover_plan(
    snapshot: &SessionInspectionSnapshot,
    now_ts: i64,
) -> Result<SessionRecoverPlan, String> {
    if snapshot.session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session_recover_not_supported: session `{}` is not a delegate child",
            snapshot.session.session_id
        ));
    }
    if snapshot.terminal_outcome.is_some() || session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is already terminal",
            snapshot.session.session_id
        ));
    }
    let lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.delegate_events.as_slice(),
        now_ts,
    )
    .ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing delegate lifecycle metadata",
            snapshot.session.session_id
        )
    })?;
    if lifecycle.mode != "async" {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is not an overdue async child",
            snapshot.session.session_id
        ));
    }
    let staleness = lifecycle.staleness.ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing staleness metadata",
            snapshot.session.session_id
        )
    })?;
    if staleness.state != "overdue" {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is not overdue",
            snapshot.session.session_id
        ));
    }
    let timeout_seconds = lifecycle.timeout_seconds.ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing timeout metadata",
            snapshot.session.session_id
        )
    })?;

    match (snapshot.session.state, lifecycle.phase) {
        (SessionState::Ready, "queued") => {
            let queued_at = lifecycle.queued_at.ok_or_else(|| {
                format!(
                    "session_recover_not_recoverable: session `{}` is missing queued timestamp",
                    snapshot.session.session_id
                )
            })?;
            Ok(SessionRecoverPlan {
                expected_state: SessionState::Ready,
                recovery_kind: RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
                reference: "queued",
                queued_at: Some(queued_at),
                started_at: lifecycle.started_at,
                elapsed_seconds: staleness.elapsed_seconds,
                timeout_seconds,
                deadline_at: staleness.deadline_at,
            })
        }
        (SessionState::Running, "running") => Ok(SessionRecoverPlan {
            expected_state: SessionState::Running,
            recovery_kind: RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED,
            reference: staleness.reference,
            queued_at: lifecycle.queued_at,
            started_at: lifecycle.started_at,
            elapsed_seconds: staleness.elapsed_seconds,
            timeout_seconds,
            deadline_at: staleness.deadline_at,
        }),
        _ => Err(format!(
            "session_recover_not_recoverable: session `{}` is not an overdue async child",
            snapshot.session.session_id
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_recovery_error(plan: &SessionRecoverPlan) -> String {
    match plan.recovery_kind {
        RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED => format!(
            "delegate_async_queued_overdue_marked_failed: queued delegate child exceeded timeout after {}s (threshold {}s)",
            plan.elapsed_seconds, plan.timeout_seconds
        ),
        RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED => format!(
            "delegate_async_running_overdue_marked_failed: running delegate child exceeded timeout after {}s (threshold {}s)",
            plan.elapsed_seconds, plan.timeout_seconds
        ),
        other => {
            format!("session_recover_unsupported_kind: unsupported session recovery kind `{other}`")
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_cancel_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
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
        10,
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
    let inspection = session_inspection_payload(snapshot.clone());
    let cancel_plan = match build_session_cancel_plan(&snapshot) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_cancellable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_cancel_action_json(&cancel_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_cancel_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        cancel_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_cancel_state_changed:") => {
            let inspection = match inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            ) {
                Ok(snapshot) => Some(session_inspection_payload(snapshot)),
                Err(_) => None,
            };
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspection,
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_cancel_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    cancel_plan: SessionCancelPlan,
) -> Result<SessionToolActionOutcome, String> {
    match cancel_plan {
        SessionCancelPlan::Queued => {
            let cancel_error = delegate_cancelled_error(DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED);
            let outcome = delegate_error_outcome(
                snapshot.session.session_id.clone(),
                snapshot.session.label.clone(),
                cancel_error.clone(),
                0,
            );
            let frozen_result =
                capture_frozen_result(&outcome, tool_config.delegate.max_frozen_bytes);
            let outcome_status = outcome.status.clone();
            let outcome_payload = outcome.payload;
            let finalized = repo.finalize_session_terminal_if_current(
                target_session_id,
                SessionState::Ready,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(cancel_error),
                    event_kind: DELEGATE_CANCELLED_EVENT_KIND.to_owned(),
                    actor_session_id: Some(current_session_id.to_owned()),
                    event_payload_json: json!({
                        "reference": "queued",
                        "cancel_reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
                    }),
                    outcome_status,
                    outcome_payload_json: outcome_payload,
                    frozen_result: Some(frozen_result),
                },
            )?;
            if finalized.is_none() {
                let latest = repo
                    .load_session_summary_with_legacy_fallback(target_session_id)?
                    .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
                return Err(format!(
                    "session_cancel_state_changed: session `{target_session_id}` is no longer cancellable from state `{}`",
                    latest.state.as_str()
                ));
            }

            let cancelled_snapshot = inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            )?;
            Ok(SessionToolActionOutcome {
                inspection: session_inspection_payload(cancelled_snapshot),
                action: session_cancel_action_json(&SessionCancelPlan::Queued),
            })
        }
        SessionCancelPlan::Running => {
            let requested = repo.transition_session_with_event_if_current(
                target_session_id,
                crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Running,
                    next_state: SessionState::Running,
                    last_error: None,
                    event_kind: DELEGATE_CANCEL_REQUESTED_EVENT_KIND.to_owned(),
                    actor_session_id: Some(current_session_id.to_owned()),
                    event_payload_json: json!({
                        "reference": "running",
                        "cancel_reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
                    }),
                },
            )?;
            if requested.is_none() {
                let latest = repo
                    .load_session_summary_with_legacy_fallback(target_session_id)?
                    .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
                return Err(format!(
                    "session_cancel_state_changed: session `{target_session_id}` is no longer cancellable from state `{}`",
                    latest.state.as_str()
                ));
            }

            let requested_snapshot = inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            )?;
            Ok(SessionToolActionOutcome {
                inspection: session_inspection_payload(requested_snapshot),
                action: session_cancel_action_json(&SessionCancelPlan::Running),
            })
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_cancel_action_json(plan: &SessionCancelPlan) -> Value {
    match plan {
        SessionCancelPlan::Queued => json!({
            "kind": "queued_async_cancelled",
            "previous_state": "ready",
            "next_state": "failed",
            "reference": "queued",
            "reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
        }),
        SessionCancelPlan::Running => json!({
            "kind": "running_async_cancel_requested",
            "previous_state": "running",
            "next_state": "running",
            "reference": "running",
            "reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
        }),
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_session_cancel_plan(
    snapshot: &SessionInspectionSnapshot,
) -> Result<SessionCancelPlan, String> {
    if snapshot.session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session_cancel_not_supported: session `{}` is not a delegate child",
            snapshot.session.session_id
        ));
    }
    if snapshot.terminal_outcome.is_some() || session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_cancel_not_cancellable: session `{}` is already terminal",
            snapshot.session.session_id
        ));
    }
    let lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.delegate_events.as_slice(),
        current_unix_ts(),
    )
    .ok_or_else(|| {
        format!(
            "session_cancel_not_cancellable: session `{}` is missing delegate lifecycle metadata",
            snapshot.session.session_id
        )
    })?;
    if lifecycle.mode != "async" {
        return Err(format!(
            "session_cancel_not_supported: session `{}` is not an async delegate child",
            snapshot.session.session_id
        ));
    }
    match (snapshot.session.state, lifecycle.phase) {
        (SessionState::Ready, "queued") => Ok(SessionCancelPlan::Queued),
        (SessionState::Running, "running") => Ok(SessionCancelPlan::Running),
        _ => Err(format!(
            "session_cancel_not_cancellable: session `{}` is not queued or running",
            snapshot.session.session_id
        )),
    }
}
