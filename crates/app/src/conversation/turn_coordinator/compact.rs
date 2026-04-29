use super::*;
use crate::conversation::ContextCompactionDiagnostics;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionReport {
    pub status: AnalyticsTurnCheckpointProgressStatus,
    pub estimated_tokens_before: Option<usize>,
    pub estimated_tokens_after: Option<usize>,
    pub diagnostics: Option<ContextCompactionDiagnostics>,
}

impl ContextCompactionReport {
    pub fn status_label(&self) -> &'static str {
        match self.status {
            AnalyticsTurnCheckpointProgressStatus::Pending => "pending",
            AnalyticsTurnCheckpointProgressStatus::Skipped => "skipped",
            AnalyticsTurnCheckpointProgressStatus::Completed => "completed",
            AnalyticsTurnCheckpointProgressStatus::Failed => "failed",
            AnalyticsTurnCheckpointProgressStatus::FailedOpen => "failed_open",
        }
    }

    pub fn was_applied(&self) -> bool {
        matches!(
            self.status,
            AnalyticsTurnCheckpointProgressStatus::Completed
        )
    }

    pub fn was_skipped(&self) -> bool {
        matches!(self.status, AnalyticsTurnCheckpointProgressStatus::Skipped)
    }

    pub fn was_failed_open(&self) -> bool {
        matches!(
            self.status,
            AnalyticsTurnCheckpointProgressStatus::FailedOpen
        )
    }
}

pub(super) async fn maybe_compact_context<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    messages: &[Value],
    estimated_tokens: Option<usize>,
    binding: ConversationRuntimeBinding<'_>,
    force: bool,
) -> CliResult<ContextCompactionOutcome> {
    let estimated_tokens = estimated_tokens.or_else(|| estimate_tokens(messages));
    let should_attempt_compaction = if force {
        true
    } else {
        config
            .conversation
            .should_compact_with_estimate(messages.len(), estimated_tokens)
    };
    if !should_attempt_compaction {
        return Ok(ContextCompactionOutcome::Skipped);
    }
    let Some(kernel_ctx) = binding.kernel_context() else {
        return Ok(ContextCompactionOutcome::Skipped);
    };

    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(error) = persist_runtime_self_continuity_for_compaction(config, session_id) {
            if config.conversation.compaction_fail_open() {
                return Ok(ContextCompactionOutcome::FailedOpen);
            }

            return Err(format!(
                "pre-compaction runtime self continuity persist failed: {error}"
            ));
        }

        let workspace_root = config
            .tools
            .file_root
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|_| config.tools.resolved_file_root());

        let memory_config = store::session_store_config_from_memory_config(&config.memory);
        let compact_stage_result =
            store::run_session_compact_stage(session_id, workspace_root.as_deref(), &memory_config)
                .await;
        match compact_stage_result {
            Ok(diagnostics)
                if matches!(diagnostics.outcome, crate::memory::StageOutcome::Fallback) =>
            {
                if config.conversation.compaction_fail_open() {
                    return Ok(ContextCompactionOutcome::FailedOpen);
                }

                return Err(format!(
                    "pre-compaction durable memory flush failed: {}",
                    diagnostics
                        .message
                        .as_deref()
                        .unwrap_or("compact stage fallback without error detail")
                ));
            }
            Ok(_) => {}
            Err(_error) if config.conversation.compaction_fail_open() => {
                return Ok(ContextCompactionOutcome::FailedOpen);
            }
            Err(error) => {
                return Err(format!(
                    "pre-compaction durable memory flush failed: {error}"
                ));
            }
        }
    }

    match runtime
        .compact_context(config, session_id, messages, kernel_ctx)
        .await
    {
        Ok(()) => Ok(ContextCompactionOutcome::Completed),
        Err(_error) if config.conversation.compaction_fail_open() => {
            Ok(ContextCompactionOutcome::FailedOpen)
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn persist_runtime_self_continuity_for_compaction(
    config: &LoongConfig,
    session_id: &str,
) -> Result<(), String> {
    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;

    ensure_session_exists_for_runtime_self_continuity(&repo, session_id)?;

    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let stored_continuity =
        runtime_self_continuity::load_persisted_runtime_self_continuity(&repo, session_id)?;
    let continuity = runtime_self_continuity::merge_runtime_self_continuity(
        live_continuity,
        stored_continuity.as_ref(),
    );
    let Some(continuity) = continuity else {
        return Ok(());
    };
    if stored_continuity.as_ref() == Some(&continuity) {
        return Ok(());
    }

    let payload = json!({
        "source": "compaction",
        "runtime_self_continuity": continuity,
    });
    let event = NewSessionEvent {
        session_id: session_id.to_owned(),
        event_kind: runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND.to_owned(),
        actor_session_id: Some(session_id.to_owned()),
        payload_json: payload,
    };
    repo.append_event(event)?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn ensure_session_exists_for_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<(), String> {
    let existing_session = repo.load_session(session_id)?;
    if existing_session.is_some() {
        return Ok(());
    }

    let summary = repo.load_session_summary_with_legacy_fallback(session_id)?;
    let delegate_parent_session_id = repo
        .list_delegate_lifecycle_events(session_id)?
        .into_iter()
        .rev()
        .find_map(|event| event.actor_session_id);
    let kind = summary.as_ref().map(|value| value.kind).unwrap_or_else(|| {
        if session_id.starts_with("delegate:") || delegate_parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        }
    });
    let parent_session_id = match kind {
        SessionKind::Root => None,
        SessionKind::DelegateChild => {
            let stored_parent_session_id = summary
                .as_ref()
                .and_then(|value| value.parent_session_id.clone());
            let reconstructed_parent_session_id =
                delegate_parent_session_id.or(stored_parent_session_id);
            let Some(reconstructed_parent_session_id) = reconstructed_parent_session_id else {
                return Err(format!(
                    "delegate session `{session_id}` is missing lineage required for runtime self continuity persistence"
                ));
            };
            Some(reconstructed_parent_session_id)
        }
    };
    let label = summary.as_ref().and_then(|value| value.label.clone());
    let state = summary
        .as_ref()
        .map(|value| value.state)
        .unwrap_or(SessionState::Ready);
    let record = NewSessionRecord {
        session_id: session_id.to_owned(),
        kind,
        parent_session_id,
        label,
        state,
    };
    let _ = repo.ensure_session(record)?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn effective_runtime_self_continuity_for_session(
    config: &LoongConfig,
    session_context: &SessionContext,
) -> Option<runtime_self_continuity::RuntimeSelfContinuity> {
    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let stored_continuity = session_context.runtime_self_continuity.as_ref();
    runtime_self_continuity::merge_runtime_self_continuity(live_continuity, stored_continuity)
}

pub(super) fn estimate_tokens(messages: &[Value]) -> Option<usize> {
    if messages.is_empty() {
        return Some(0);
    }

    let estimated = messages.iter().fold(0usize, |acc, message| {
        let role_chars = message
            .get("role")
            .map_or(0usize, |value| value.to_string().chars().count());
        let content_chars = message
            .get("content")
            .map_or(0usize, |value| value.to_string().chars().count());
        let token_estimate = (role_chars + content_chars).div_ceil(4) + 4;
        acc.saturating_add(token_estimate)
    });

    Some(estimated)
}

pub(super) fn analytics_turn_checkpoint_progress_status(
    status: TurnCheckpointProgressStatus,
) -> AnalyticsTurnCheckpointProgressStatus {
    match status {
        TurnCheckpointProgressStatus::Pending => AnalyticsTurnCheckpointProgressStatus::Pending,
        TurnCheckpointProgressStatus::Skipped => AnalyticsTurnCheckpointProgressStatus::Skipped,
        TurnCheckpointProgressStatus::Completed => AnalyticsTurnCheckpointProgressStatus::Completed,
        TurnCheckpointProgressStatus::Failed => AnalyticsTurnCheckpointProgressStatus::Failed,
        TurnCheckpointProgressStatus::FailedOpen => {
            AnalyticsTurnCheckpointProgressStatus::FailedOpen
        }
    }
}
