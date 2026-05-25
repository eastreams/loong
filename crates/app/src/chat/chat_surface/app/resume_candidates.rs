#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeCandidate {
    pub(crate) session_id: String,
    pub(crate) last_user_turn_at: i64,
    pub(crate) preview_text: String,
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_resume_candidates(
    current_session_id: &str,
    config: &crate::session::store::SessionStoreConfig,
) -> Result<Vec<ResumeCandidate>, String> {
    let repo = crate::session::repository::SessionRepository::new(config)?;
    let mut candidates = Vec::new();

    for session in repo.list_sessions()? {
        if session.kind != crate::session::repository::SessionKind::Root {
            continue;
        }
        if session.session_id == current_session_id {
            continue;
        }
        let Some(summary) = repo.load_session_summary(&session.session_id)? else {
            continue;
        };
        if summary.archived_at.is_some() {
            continue;
        }

        let turns = crate::session::store::window_session_turns(&summary.session_id, 64, config)?;
        let Some(turn) = turns.iter().rev().find(|turn| turn.role == "user") else {
            continue;
        };

        candidates.push(ResumeCandidate {
            session_id: summary.session_id,
            last_user_turn_at: turn.ts,
            preview_text: turn.content.chars().take(20).collect(),
        });
    }

    candidates.sort_by(|left, right| {
        right
            .last_user_turn_at
            .cmp(&left.last_user_turn_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(candidates)
}
