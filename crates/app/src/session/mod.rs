#[cfg(feature = "memory-sqlite")]
pub mod recovery;

#[cfg(feature = "memory-sqlite")]
pub mod repository;

#[cfg(feature = "memory-sqlite")]
pub mod store;

#[cfg(feature = "memory-sqlite")]
pub mod trajectory;

#[cfg(feature = "memory-sqlite")]
pub mod frozen_result;

pub const LATEST_SESSION_SELECTOR: &str = "latest";

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRootSessionCandidate {
    pub session_id: String,
    pub last_user_turn_at: i64,
    pub preview_text: String,
}

#[cfg(feature = "memory-sqlite")]
pub fn latest_resumable_root_session_id(
    store_config: &store::SessionStoreConfig,
) -> crate::CliResult<Option<String>> {
    let repo = repository::SessionRepository::new(store_config)?;
    let latest_session = repo.latest_resumable_root_session_summary()?;
    let latest_session_id = latest_session.map(|summary| summary.session_id);
    Ok(latest_session_id)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn resume_candidates_for_root_sessions(
    store_config: &store::SessionStoreConfig,
    current_session_id: &str,
) -> crate::CliResult<Vec<ResumeRootSessionCandidate>> {
    let repo = repository::SessionRepository::new(store_config)?;
    let sessions = repo.list_resumable_root_session_summaries()?;
    let mut candidates = Vec::new();

    for session in sessions {
        if session.session_id == current_session_id {
            continue;
        }

        let Some(turn) =
            store::latest_session_turn_by_role(&session.session_id, "user", store_config)?
        else {
            continue;
        };

        candidates.push(ResumeRootSessionCandidate {
            session_id: session.session_id,
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

#[cfg(feature = "memory-sqlite")]
pub fn created_empty_sessions_for_cleanup(
    store_config: &store::SessionStoreConfig,
    created_this_run_ids: &[String],
) -> crate::CliResult<Vec<String>> {
    let repo = repository::SessionRepository::new(store_config)?;
    let mut doomed = Vec::new();

    for session_id in created_this_run_ids {
        let Some(_session) = repo.load_session(session_id)? else {
            continue;
        };
        if store::latest_session_turn_by_role(session_id, "user", store_config)?.is_none() {
            doomed.push(session_id.clone());
        }
    }

    Ok(doomed)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn delete_session_by_id(
    store_config: &store::SessionStoreConfig,
    session_id: &str,
) -> crate::CliResult<bool> {
    let repo = repository::SessionRepository::new(store_config)?;
    repo.delete_session(session_id)
}

pub(crate) const DELEGATE_CANCEL_REQUESTED_EVENT_KIND: &str = "delegate_cancel_requested";
pub(crate) const DELEGATE_CANCELLED_EVENT_KIND: &str = "delegate_cancelled";
pub(crate) const DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED: &str = "operator_requested";
const DELEGATE_CANCELLED_ERROR_PREFIX: &str = "delegate_cancelled:";

pub(crate) fn delegate_cancelled_error(reason: &str) -> String {
    format!(
        "{DELEGATE_CANCELLED_ERROR_PREFIX} {}",
        reason.trim().trim_matches(':')
    )
}

pub(crate) fn parse_delegate_cancelled_reason(error: &str) -> Option<String> {
    let trimmed_error = error.trim();
    let cancelled_prefix = DELEGATE_CANCELLED_ERROR_PREFIX;
    let raw_reason = trimmed_error.strip_prefix(cancelled_prefix)?;
    let trimmed_reason = raw_reason.trim();
    if trimmed_reason.is_empty() {
        return None;
    }

    Some(trimmed_reason.to_owned())
}

#[cfg(test)]
mod delegate_cancelled_reason_tests {
    use super::delegate_cancelled_error;
    use super::parse_delegate_cancelled_reason;

    #[test]
    fn parse_delegate_cancelled_reason_extracts_trimmed_reason() {
        let error = delegate_cancelled_error("operator_requested");
        let parsed_reason = parse_delegate_cancelled_reason(&error);

        assert_eq!(parsed_reason.as_deref(), Some("operator_requested"));
    }

    #[test]
    fn parse_delegate_cancelled_reason_rejects_non_cancelled_errors() {
        let parsed_reason = parse_delegate_cancelled_reason("delegate_timeout");

        assert_eq!(parsed_reason, None);
    }
}

#[cfg(all(test, feature = "memory-sqlite"))]
#[allow(clippy::expect_used)]
mod latest_cli_session_selector_tests {
    use super::created_empty_sessions_for_cleanup;
    use super::LATEST_SESSION_SELECTOR;
    use super::latest_resumable_root_session_id;
    use super::resume_candidates_for_root_sessions;
    use crate::session::repository::NewSessionRecord;
    use crate::session::repository::SessionKind;
    use crate::session::repository::SessionRepository;
    use crate::session::repository::SessionState;
    use crate::session::store;
    use crate::test_support::unique_temp_dir;
    use rusqlite::Connection;
    use rusqlite::params;
    use std::path::Path;
    use std::path::PathBuf;

    fn init_selector_test_memory(label: &str) -> (PathBuf, store::SessionStoreConfig) {
        let root = unique_temp_dir(label);
        std::fs::create_dir_all(&root).expect("create selector test workspace");

        let sqlite_path = root.join("memory.sqlite3");
        let config = store::SessionStoreConfig {
            sqlite_path: Some(sqlite_path.clone()),
            runtime_config: None,
        };

        store::ensure_session_store_ready(Some(sqlite_path), &config)
            .expect("initialize selector test memory");

        (root, config)
    }

    fn cleanup_selector_test_memory(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn create_root_session(repo: &SessionRepository, session_id: &str) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
    }

    fn append_session_turn(
        store_config: &store::SessionStoreConfig,
        session_id: &str,
        role: &str,
        content: &str,
    ) {
        store::append_session_turn_direct(session_id, role, content, store_config)
            .expect("append selector test turn");
    }

    fn set_session_updated_at(sqlite_path: &Path, session_id: &str, updated_at: i64) {
        let conn = Connection::open(sqlite_path).expect("open selector sqlite connection");
        conn.execute(
            "UPDATE sessions
             SET updated_at = ?2
             WHERE session_id = ?1",
            params![session_id, updated_at],
        )
        .expect("set selector updated_at");
    }

    fn archive_session(sqlite_path: &Path, session_id: &str, archived_at: i64) {
        let conn = Connection::open(sqlite_path).expect("open selector sqlite connection");
        conn.execute(
            "INSERT INTO session_events(
                session_id,
                event_kind,
                actor_session_id,
                payload_json,
                ts
             ) VALUES (?1, ?2, NULL, ?3, ?4)",
            params![session_id, "session_archived", "{}", archived_at],
        )
        .expect("archive selector session");
    }

    #[test]
    fn latest_cli_session_selector_returns_newest_resumable_root_session_id() {
        let (root, memory_config) = init_selector_test_memory("latest-cli-selector");
        let sqlite_path = memory_config
            .sqlite_path
            .clone()
            .expect("selector sqlite path");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        assert_eq!(LATEST_SESSION_SELECTOR, "latest");

        create_root_session(&repo, "root-old");
        append_session_turn(&memory_config, "root-old", "user", "old");
        set_session_updated_at(&sqlite_path, "root-old", 100);

        create_root_session(&repo, "root-new");
        append_session_turn(&memory_config, "root-new", "user", "new");
        set_session_updated_at(&sqlite_path, "root-new", 200);

        create_root_session(&repo, "root-archived");
        append_session_turn(&memory_config, "root-archived", "assistant", "archived");
        set_session_updated_at(&sqlite_path, "root-archived", 300);
        archive_session(&sqlite_path, "root-archived", 400);

        let selected_session_id = latest_resumable_root_session_id(&memory_config)
            .expect("resolve latest session id")
            .expect("selected session id");

        assert_eq!(selected_session_id, "root-new");

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn latest_cli_session_selector_returns_none_without_resumable_root_session() {
        let (root, memory_config) = init_selector_test_memory("latest-cli-selector-none");
        let sqlite_path = memory_config
            .sqlite_path
            .clone()
            .expect("selector sqlite path");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        create_root_session(&repo, "root-empty");
        set_session_updated_at(&sqlite_path, "root-empty", 100);

        create_root_session(&repo, "root-archived");
        append_session_turn(&memory_config, "root-archived", "assistant", "archived");
        set_session_updated_at(&sqlite_path, "root-archived", 200);
        archive_session(&sqlite_path, "root-archived", 300);

        let selected_session_id =
            latest_resumable_root_session_id(&memory_config).expect("resolve latest session id");

        assert!(selected_session_id.is_none());

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn startup_created_root_session_is_not_latest_eligible_without_user_turn() {
        let (root, memory_config) = init_selector_test_memory("startup-created-empty");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        repo.create_session(NewSessionRecord {
            session_id: "startup-empty".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("startup-empty".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create startup-empty");

        assert_eq!(
            latest_resumable_root_session_id(&memory_config).expect("resolve latest"),
            None
        );

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn resume_candidates_exclude_created_empty_root_sessions_without_user_turns() {
        let (root, memory_config) = init_selector_test_memory("resume-empty-filter");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        repo.create_session(NewSessionRecord {
            session_id: "current-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("current-session".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create current-session");

        repo.create_session(NewSessionRecord {
            session_id: "empty-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: Some("current-session".to_owned()),
            label: Some("empty-root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create empty-root");

        repo.create_session(NewSessionRecord {
            session_id: "root-user".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: Some("current-session".to_owned()),
            label: Some("root-user".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root-user");
        append_session_turn(&memory_config, "root-user", "user", "hello from root-user");

        repo.create_session(NewSessionRecord {
            session_id: "unrelated-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("unrelated-root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create unrelated-root");
        append_session_turn(
            &memory_config,
            "unrelated-root",
            "user",
            "hello from unrelated-root",
        );

        let candidates = resume_candidates_for_root_sessions(&memory_config, "current-session")
            .expect("load candidates");

        let ids = candidates
            .iter()
            .map(|candidate| candidate.session_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["root-user"]);

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn resume_candidates_use_visibility_scope_for_legacy_current_session() {
        let (root, memory_config) = init_selector_test_memory("resume-legacy-visibility");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        append_session_turn(&memory_config, "legacy-current", "user", "legacy current");

        repo.create_session(NewSessionRecord {
            session_id: "unrelated-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("unrelated-root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create unrelated-root");
        append_session_turn(&memory_config, "unrelated-root", "user", "visible elsewhere");

        let candidates = resume_candidates_for_root_sessions(&memory_config, "legacy-current")
            .expect("load candidates");

        assert!(candidates.is_empty());

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn created_this_run_empty_sessions_are_selected_for_exit_cleanup() {
        let (root, memory_config) = init_selector_test_memory("cleanup-empty-created");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        for session_id in ["created-empty", "created-with-user", "resumed-existing"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::Root,
                parent_session_id: None,
                label: Some(session_id.to_owned()),
                state: SessionState::Ready,
            })
            .expect("create session");
        }

        append_session_turn(&memory_config, "created-with-user", "user", "keep me");

        let doomed = created_empty_sessions_for_cleanup(
            &memory_config,
            &["created-empty".to_owned(), "created-with-user".to_owned()],
        )
        .expect("select cleanup sessions");

        assert_eq!(doomed, vec!["created-empty".to_owned()]);
        assert!(
            repo.load_session("resumed-existing")
                .expect("load session")
                .is_some()
        );

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn resume_candidates_keep_sessions_with_old_user_turn_outside_recent_window() {
        let (root, memory_config) = init_selector_test_memory("resume-old-user-turn");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        repo.create_session(NewSessionRecord {
            session_id: "current-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("current-session".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create current-session");

        repo.create_session(NewSessionRecord {
            session_id: "root-user-buried".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: Some("current-session".to_owned()),
            label: Some("root-user-buried".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root-user-buried");
        append_session_turn(
            &memory_config,
            "root-user-buried",
            "user",
            "hello from buried root-user",
        );
        for index in 0..80 {
            append_session_turn(
                &memory_config,
                "root-user-buried",
                "assistant",
                &format!("assistant follow-up {index}"),
            );
        }

        let candidates = resume_candidates_for_root_sessions(&memory_config, "current-session")
            .expect("load candidates");
        let ids = candidates
            .iter()
            .map(|candidate| candidate.session_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["root-user-buried"]);

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn resume_candidates_include_other_independent_root_sessions() {
        let (root, memory_config) = init_selector_test_memory("resume-independent-roots");
        let sqlite_path = memory_config
            .sqlite_path
            .clone()
            .expect("selector sqlite path");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        create_root_session(&repo, "current-root");
        append_session_turn(&memory_config, "current-root", "user", "current root turn");
        set_session_updated_at(&sqlite_path, "current-root", 100);

        create_root_session(&repo, "other-root-new");
        append_session_turn(&memory_config, "other-root-new", "user", "other newer turn");
        set_session_updated_at(&sqlite_path, "other-root-new", 300);

        create_root_session(&repo, "other-root-old");
        append_session_turn(&memory_config, "other-root-old", "user", "other older turn");
        set_session_updated_at(&sqlite_path, "other-root-old", 200);

        let candidates = resume_candidates_for_root_sessions(&memory_config, "current-root")
            .expect("load candidates");
        let ids = candidates
            .iter()
            .map(|candidate| candidate.session_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["other-root-new", "other-root-old"]);

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn cleanup_keeps_sessions_with_old_user_turn_outside_recent_window() {
        let (root, memory_config) = init_selector_test_memory("cleanup-old-user-turn");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        repo.create_session(NewSessionRecord {
            session_id: "created-with-buried-user".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("created-with-buried-user".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create created-with-buried-user");

        append_session_turn(
            &memory_config,
            "created-with-buried-user",
            "user",
            "keep me even if old",
        );
        for index in 0..80 {
            append_session_turn(
                &memory_config,
                "created-with-buried-user",
                "assistant",
                &format!("assistant follow-up {index}"),
            );
        }

        let doomed = created_empty_sessions_for_cleanup(
            &memory_config,
            &["created-with-buried-user".to_owned()],
        )
        .expect("select cleanup sessions");

        assert!(doomed.is_empty());

        cleanup_selector_test_memory(&root);
    }
}
