use std::path::Path;

use crate::config::MemoryMode;

use super::{
    DerivedMemoryKind, MemoryContextEntry, MemoryContextKind, MemoryRetrievalRequest, MemoryScope,
    WindowTurn, runtime_config::MemoryRuntimeConfig,
};

pub struct MemoryPreAssemblyContext<'a> {
    pub session_id: &'a str,
    pub workspace_root: Option<&'a Path>,
    pub config: &'a MemoryRuntimeConfig,
    pub recent_window: &'a [WindowTurn],
}

pub trait MemoryPreAssemblyExecutor: Send + Sync {
    fn retrieval_request(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Option<MemoryRetrievalRequest> {
        let _ = context;
        None
    }

    fn derive(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        let _ = context;
        Ok(Vec::new())
    }

    fn retrieve(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        let _ = context;
        Ok(Vec::new())
    }

    fn rank(
        &self,
        entries: Vec<MemoryContextEntry>,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        let _ = context;
        Ok(entries)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinMemoryPreAssemblyExecutor;

impl MemoryPreAssemblyExecutor for BuiltinMemoryPreAssemblyExecutor {
    fn retrieval_request(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Option<MemoryRetrievalRequest> {
        builtin_retrieval_request(context)
    }

    fn retrieve(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        run_builtin_retrieval_entries(context)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RecallFirstMemoryPreAssemblyExecutor;

impl MemoryPreAssemblyExecutor for RecallFirstMemoryPreAssemblyExecutor {
    fn retrieval_request(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Option<MemoryRetrievalRequest> {
        builtin_retrieval_request(context)
    }

    fn retrieve(
        &self,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        run_builtin_retrieval_entries(context)
    }

    fn rank(
        &self,
        entries: Vec<MemoryContextEntry>,
        context: &MemoryPreAssemblyContext<'_>,
    ) -> Result<Vec<MemoryContextEntry>, String> {
        let _ = context;
        let ranked_entries = rank_recall_first_entries(entries);
        Ok(ranked_entries)
    }
}

fn builtin_retrieval_request(
    context: &MemoryPreAssemblyContext<'_>,
) -> Option<MemoryRetrievalRequest> {
    let mode = context.config.mode;
    let supports_recall = matches!(mode, MemoryMode::WindowPlusSummary);
    if !supports_recall {
        return None;
    }

    let query = retrieval_query_from_recent_window(context.recent_window);

    let has_recent_window = !context.recent_window.is_empty();
    let budget_items = if has_recent_window {
        context.config.sliding_window.min(6)
    } else {
        6
    };

    let retrieval_request = MemoryRetrievalRequest {
        session_id: context.session_id.to_owned(),
        query,
        scopes: vec![
            MemoryScope::Session,
            MemoryScope::Workspace,
            MemoryScope::Agent,
            MemoryScope::User,
        ],
        budget_items,
        allowed_kinds: vec![
            DerivedMemoryKind::Profile,
            DerivedMemoryKind::Fact,
            DerivedMemoryKind::Episode,
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Overview,
        ],
    };

    Some(retrieval_request)
}

pub(crate) fn retrieval_query_from_recent_window(recent_window: &[WindowTurn]) -> Option<String> {
    let reversed_turns = recent_window.iter().rev();

    for turn in reversed_turns {
        let role = turn.role.as_str();
        if role != "user" {
            continue;
        }

        let content = turn.content.as_str();
        let trimmed_content = content.trim();
        if trimmed_content.is_empty() {
            continue;
        }

        let query = trimmed_content.to_owned();
        return Some(query);
    }

    None
}

fn run_builtin_retrieval_entries(
    context: &MemoryPreAssemblyContext<'_>,
) -> Result<Vec<MemoryContextEntry>, String> {
    let workspace_root = context.workspace_root;
    let config = context.config;
    let mut entries = super::load_durable_recall_entries(workspace_root, config)?;

    #[cfg(feature = "memory-sqlite")]
    {
        let query = retrieval_query_from_recent_window(context.recent_window);
        if let Some(query) = query {
            let budget_items = context.config.sliding_window.min(6);
            let hits = super::search_canonical_memory(
                query.as_str(),
                budget_items,
                Some(context.session_id),
                context.config,
            )?;
            if !hits.is_empty() {
                let content = render_cross_session_recall_block(hits.as_slice());
                let recall_entry = MemoryContextEntry {
                    kind: MemoryContextKind::RetrievedMemory,
                    role: "system".to_owned(),
                    content,
                };
                entries.push(recall_entry);
            }
        }
    }

    Ok(entries)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn render_cross_session_recall_block(
    hits: &[super::CanonicalMemorySearchHit],
) -> String {
    let mut sections = Vec::new();
    let heading = "## Advisory Cross-Session Recall".to_owned();
    let intro =
        "These snippets were retrieved from prior persisted sessions. Treat them as advisory hints and verify before acting."
            .to_owned();

    sections.push(heading);
    sections.push(intro);

    for hit in hits {
        let turn_label = hit
            .session_turn_index
            .map(|value| format!("turn {value}"))
            .unwrap_or_else(|| "turn ?".to_owned());
        let role_label = hit.record.role.as_deref();
        let content = hit.record.content.as_str();
        let truncated_content = truncate_recall_content(content, 280);
        let entry_heading = format!(
            "### {} · {} · {} · {}",
            hit.record.session_id,
            turn_label,
            hit.record.scope.as_str(),
            hit.record.kind.as_str()
        );
        let recall_line = match role_label {
            Some(role_label) => format!("{role_label}: {truncated_content}"),
            None => truncated_content,
        };

        sections.push(entry_heading);
        sections.push(recall_line);
    }

    sections.join("\n\n")
}

#[cfg(feature = "memory-sqlite")]
fn truncate_recall_content(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_owned();
    }
    if max_chars <= 3 {
        let truncated = input.chars().take(max_chars).collect();
        return truncated;
    }

    let prefix_length = max_chars - 3;
    let prefix = input.chars().take(prefix_length).collect::<String>();
    let truncated = format!("{prefix}...");
    truncated
}

fn rank_recall_first_entries(entries: Vec<MemoryContextEntry>) -> Vec<MemoryContextEntry> {
    let has_retrieved_memory = entries
        .iter()
        .any(|entry| entry.kind == MemoryContextKind::RetrievedMemory);

    let mut retrieved_entries = Vec::new();
    let mut profile_entries = Vec::new();
    let mut summary_entries = Vec::new();
    let mut turn_entries = Vec::new();

    for entry in entries {
        let kind = entry.kind;
        match kind {
            MemoryContextKind::RetrievedMemory => {
                retrieved_entries.push(entry);
            }
            MemoryContextKind::Profile => {
                profile_entries.push(entry);
            }
            MemoryContextKind::Summary => {
                if !has_retrieved_memory {
                    summary_entries.push(entry);
                }
            }
            MemoryContextKind::Turn => {
                turn_entries.push(entry);
            }
        }
    }

    let mut ranked_entries = Vec::new();
    ranked_entries.extend(retrieved_entries);
    ranked_entries.extend(profile_entries);
    ranked_entries.extend(summary_entries);
    ranked_entries.extend(turn_entries);
    ranked_entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_query_prefers_latest_non_empty_user_turn() {
        let recent_window = vec![
            WindowTurn {
                role: "assistant".to_owned(),
                content: "ignored".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "  ".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "latest query".to_owned(),
                ts: None,
            },
        ];

        let query = retrieval_query_from_recent_window(recent_window.as_slice());

        assert_eq!(query.as_deref(), Some("latest query"));
    }

    #[test]
    fn recall_first_ranker_suppresses_summary_when_retrieved_memory_exists() {
        let entries = vec![
            MemoryContextEntry {
                kind: MemoryContextKind::Summary,
                role: "system".to_owned(),
                content: "summary".to_owned(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::Turn,
                role: "user".to_owned(),
                content: "turn".to_owned(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::RetrievedMemory,
                role: "system".to_owned(),
                content: "recall".to_owned(),
            },
        ];

        let ranked_entries = rank_recall_first_entries(entries);
        let ranked_kinds = ranked_entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            ranked_kinds,
            vec![MemoryContextKind::RetrievedMemory, MemoryContextKind::Turn]
        );
    }
}
