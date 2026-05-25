#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeCandidate {
    pub(crate) session_id: String,
    pub(crate) last_user_turn_at: i64,
    pub(crate) preview_text: String,
}

#[cfg(feature = "memory-sqlite")]
// Thin adapter that keeps TUI-local DTOs out of the session-layer contract.
pub(crate) fn load_resume_candidates(
    current_session_id: &str,
    config: &crate::session::store::SessionStoreConfig,
) -> Result<Vec<ResumeCandidate>, String> {
    crate::session::resume_candidates_for_root_sessions(config, current_session_id).map(
        |candidates| {
            candidates
                .into_iter()
                .map(|candidate| ResumeCandidate {
                    session_id: candidate.session_id,
                    last_user_turn_at: candidate.last_user_turn_at,
                    preview_text: candidate.preview_text,
                })
                .collect()
        },
    )
}
