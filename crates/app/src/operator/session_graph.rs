#[cfg(feature = "memory-sqlite")]
use crate::session::repository::SessionRepository;

#[cfg(feature = "memory-sqlite")]
pub(crate) struct OperatorSessionGraph<'a> {
    repo: &'a SessionRepository,
}

#[cfg(feature = "memory-sqlite")]
impl<'a> OperatorSessionGraph<'a> {
    pub(crate) fn new(repo: &'a SessionRepository) -> Self {
        Self { repo }
    }

    pub(crate) fn lineage_depth(&self, session_id: &str) -> Result<usize, String> {
        self.repo.session_lineage_depth(session_id)
    }

    pub(crate) fn lineage_root_session_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        self.repo.lineage_root_session_id(session_id)
    }

    pub(crate) fn effective_lineage_root_session_id(
        &self,
        session_id: &str,
        parent_session_id: Option<&str>,
    ) -> Result<String, String> {
        let stored_lineage_root_session_id = match self.lineage_root_session_id(session_id) {
            Ok(lineage_root_session_id) => lineage_root_session_id,
            Err(error) if error.starts_with("session_lineage_broken:") => None,
            Err(error) => return Err(error),
        };

        if let Some(stored_lineage_root_session_id) = stored_lineage_root_session_id {
            return Ok(stored_lineage_root_session_id);
        }

        let Some(parent_session_id) = parent_session_id else {
            return Ok(session_id.to_owned());
        };

        let stored_parent_lineage_root_session_id =
            match self.lineage_root_session_id(parent_session_id) {
                Ok(lineage_root_session_id) => lineage_root_session_id,
                Err(error) if error.starts_with("session_lineage_broken:") => None,
                Err(error) => return Err(error),
            };

        if let Some(stored_parent_lineage_root_session_id) = stored_parent_lineage_root_session_id {
            return Ok(stored_parent_lineage_root_session_id);
        }

        Ok(parent_session_id.to_owned())
    }

    pub(crate) fn next_delegate_child_depth(
        &self,
        session_id: &str,
        max_depth: usize,
    ) -> Result<usize, String> {
        let current_depth = self.lineage_depth(session_id)?;
        let next_child_depth = current_depth.saturating_add(1);

        if next_child_depth > max_depth {
            let error = format!(
                "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {max_depth}"
            );
            return Err(error);
        }

        Ok(next_child_depth)
    }
}

#[cfg(test)]
mod tests {
    use super::OperatorSessionGraph;

    use rusqlite::params;

    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        NewSessionRecord, SessionKind, SessionRepository, SessionState,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let process_id = std::process::id();
        let temp_dir = std::env::temp_dir();
        let directory_name = format!("loongclaw-operator-session-graph-{test_name}-{process_id}");
        let base_dir = temp_dir.join(directory_name);
        let _ = std::fs::create_dir_all(&base_dir);

        let db_path = base_dir.join("memory.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn delete_session_row(memory_config: &MemoryRuntimeConfig, session_id: &str) {
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path should be configured");
        let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite connection");

        conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )
        .expect("delete session row");
    }

    fn seed_session(
        repo: &SessionRepository,
        session_id: &str,
        kind: SessionKind,
        parent_session_id: Option<&str>,
    ) {
        let session_record = NewSessionRecord {
            session_id: session_id.to_owned(),
            kind,
            parent_session_id: parent_session_id.map(str::to_owned),
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        };

        repo.create_session(session_record).expect("create session");
    }

    #[test]
    fn operator_session_graph_returns_lineage_root_for_delegate_descendant() {
        let memory_config = isolated_memory_config("lineage-root");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        seed_session(
            &repo,
            "grandchild-session",
            SessionKind::DelegateChild,
            Some("child-session"),
        );

        let session_graph = OperatorSessionGraph::new(&repo);
        let lineage_root_session_id = session_graph
            .lineage_root_session_id("grandchild-session")
            .expect("compute lineage root");

        assert_eq!(lineage_root_session_id.as_deref(), Some("root-session"));
    }

    #[test]
    fn operator_session_graph_falls_back_to_session_id_when_root_row_is_missing() {
        let memory_config = isolated_memory_config("missing-root-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");
        let session_graph = OperatorSessionGraph::new(&repo);

        let effective_lineage_root_session_id = session_graph
            .effective_lineage_root_session_id("root-session", None)
            .expect("compute effective lineage root");

        assert_eq!(effective_lineage_root_session_id, "root-session");
    }

    #[test]
    fn operator_session_graph_falls_back_to_parent_scope_when_child_row_is_missing() {
        let memory_config = isolated_memory_config("missing-child-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);

        let session_graph = OperatorSessionGraph::new(&repo);
        let effective_lineage_root_session_id = session_graph
            .effective_lineage_root_session_id("child-session", Some("root-session"))
            .expect("compute effective lineage root");

        assert_eq!(effective_lineage_root_session_id, "root-session");
    }

    #[test]
    fn operator_session_graph_falls_back_to_parent_scope_when_parent_row_is_missing() {
        let memory_config = isolated_memory_config("missing-parent-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        delete_session_row(&memory_config, "root-session");

        let session_graph = OperatorSessionGraph::new(&repo);
        let effective_lineage_root_session_id = session_graph
            .effective_lineage_root_session_id("child-session", Some("root-session"))
            .expect("compute effective lineage root");

        assert_eq!(effective_lineage_root_session_id, "root-session");
    }
}
