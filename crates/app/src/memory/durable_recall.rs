use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::runtime_self_continuity;

use super::{
    MemoryContextEntry, MemoryContextKind, MemoryContextProvenance, MemoryRecallMode, MemoryScope,
    WorkspaceMemoryDocumentKind, WorkspaceMemoryDocumentLocation,
    collect_workspace_memory_document_locations, parse_workspace_memory_document,
    runtime_config::MemoryRuntimeConfig,
};

const RECENT_DAILY_LOG_LIMIT: usize = 2;
const DURABLE_RECALL_READ_SLACK_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DurableRecallDocument {
    pub label: String,
    pub content: String,
    pub provenance: MemoryContextProvenance,
}

pub(crate) fn load_durable_recall_entries(
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
) -> Result<Vec<MemoryContextEntry>, String> {
    let Some(workspace_root) = workspace_root else {
        return Ok(Vec::new());
    };

    let per_file_char_budget = config.summary_max_chars.max(256);
    let documents = collect_durable_recall_documents(
        workspace_root,
        per_file_char_budget,
        memory_system_id,
        recall_mode,
    )?;
    if documents.is_empty() {
        return Ok(Vec::new());
    }

    let content = render_durable_recall_block(documents.as_slice());
    let provenance = documents
        .iter()
        .map(|document| document.provenance.clone())
        .collect::<Vec<_>>();
    let entry = MemoryContextEntry {
        kind: MemoryContextKind::RetrievedMemory,
        role: "system".to_owned(),
        content,
        provenance,
    };

    Ok(vec![entry])
}

pub(crate) fn load_workspace_document_recall_entries(
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
    scopes: &[MemoryScope],
    max_documents: usize,
) -> Result<Vec<MemoryContextEntry>, String> {
    let Some(workspace_root) = workspace_root else {
        return Ok(Vec::new());
    };

    let per_file_char_budget = config.summary_max_chars.max(256);
    let documents = collect_durable_recall_documents(
        workspace_root,
        per_file_char_budget,
        memory_system_id,
        recall_mode,
    )?;
    if documents.is_empty() {
        return Ok(Vec::new());
    }

    let filtered_documents = filter_recall_documents_by_scope(documents, scopes);
    let capped_documents = filtered_documents
        .into_iter()
        .take(max_documents)
        .collect::<Vec<_>>();
    let mut entries = Vec::new();

    for document in capped_documents {
        let heading = format!("## Advisory Durable Recall — {}", document.label);
        let intro = runtime_self_continuity::runtime_durable_recall_intro().to_owned();
        let content = [heading, intro, document.content.clone()].join("\n\n");
        let entry = MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content,
            provenance: vec![document.provenance.clone()],
        };
        entries.push(entry);
    }

    Ok(entries)
}

fn filter_recall_documents_by_scope(
    documents: Vec<DurableRecallDocument>,
    scopes: &[MemoryScope],
) -> Vec<DurableRecallDocument> {
    if scopes.is_empty() {
        return documents;
    }

    documents
        .into_iter()
        .filter(|document| {
            let scope = document.provenance.scope.unwrap_or(MemoryScope::Workspace);
            scopes.contains(&scope)
        })
        .collect()
}

pub(crate) fn collect_durable_recall_documents(
    workspace_root: &Path,
    per_file_char_budget: usize,
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
) -> Result<Vec<DurableRecallDocument>, String> {
    let document_locations = collect_workspace_memory_document_locations(workspace_root)?;
    let mut documents = Vec::new();

    let curated_locations = document_locations
        .iter()
        .filter(|location| location.kind == WorkspaceMemoryDocumentKind::Curated);
    for location in curated_locations {
        let maybe_document = load_document_from_location(
            location,
            per_file_char_budget,
            memory_system_id,
            recall_mode,
        )?;
        let Some(document) = maybe_document else {
            continue;
        };
        documents.push(document);
    }

    let daily_locations = document_locations
        .iter()
        .filter(|location| location.kind == WorkspaceMemoryDocumentKind::DailyLog)
        .take(RECENT_DAILY_LOG_LIMIT);
    for location in daily_locations {
        let maybe_document = load_document_from_location(
            location,
            per_file_char_budget,
            memory_system_id,
            recall_mode,
        )?;
        let Some(document) = maybe_document else {
            continue;
        };
        documents.push(document);
    }

    Ok(documents)
}

fn load_document_from_location(
    location: &WorkspaceMemoryDocumentLocation,
    per_file_char_budget: usize,
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
) -> Result<Option<DurableRecallDocument>, String> {
    let path = location.path.as_path();
    let maybe_raw_content = load_trimmed_document_content(path, per_file_char_budget)?;
    let Some(raw_content) = maybe_raw_content else {
        return Ok(None);
    };

    let maybe_parsed_document = parse_workspace_memory_document(
        raw_content.as_str(),
        location,
        memory_system_id,
        recall_mode,
    )?;
    let Some(parsed_document) = maybe_parsed_document else {
        return Ok(None);
    };

    let record_status = parsed_document
        .provenance
        .record_status
        .unwrap_or(super::MemoryRecordStatus::Active);
    if !record_status.is_active() {
        return Ok(None);
    }

    let document = DurableRecallDocument {
        label: location.label.clone(),
        content: parsed_document.body,
        provenance: parsed_document.provenance,
    };

    Ok(Some(document))
}

fn load_trimmed_document_content(
    path: &Path,
    per_file_char_budget: usize,
) -> Result<Option<String>, String> {
    let read_limit = per_file_char_budget.saturating_add(DURABLE_RECALL_READ_SLACK_BYTES);
    let file = File::open(path).map_err(|error| {
        format!(
            "read durable recall file {} failed: {error}",
            path.display()
        )
    })?;
    let reader = BufReader::new(file);
    let mut limited_reader = reader.take(read_limit as u64);
    let mut raw_bytes = Vec::new();

    limited_reader
        .read_to_end(&mut raw_bytes)
        .map_err(|error| {
            format!(
                "read durable recall file {} failed: {error}",
                path.display()
            )
        })?;

    let raw_content = String::from_utf8(raw_bytes).map_err(|error| {
        format!(
            "read durable recall file {} failed: {error}",
            path.display()
        )
    })?;

    let trimmed_content = raw_content.trim();
    if trimmed_content.is_empty() {
        return Ok(None);
    }

    let bounded_content = truncate_chars(trimmed_content, per_file_char_budget);
    Ok(Some(bounded_content))
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut removed_chars = char_count.saturating_sub(max_chars);
    loop {
        let suffix = format!("...(truncated {removed_chars} chars)");
        let suffix_char_count = suffix.chars().count();
        if suffix_char_count >= max_chars {
            return suffix.chars().take(max_chars).collect();
        }

        let kept_chars = max_chars.saturating_sub(suffix_char_count);
        let next_removed_chars = char_count.saturating_sub(kept_chars);
        if next_removed_chars == removed_chars {
            let prefix = input.chars().take(kept_chars).collect::<String>();
            return format!("{prefix}{suffix}");
        }

        removed_chars = next_removed_chars;
    }
}

pub(crate) fn render_durable_recall_block(documents: &[DurableRecallDocument]) -> String {
    let mut sections = Vec::new();
    sections.push("## Advisory Durable Recall".to_owned());
    sections.push(runtime_self_continuity::runtime_durable_recall_intro().to_owned());

    for document in documents {
        let heading = format!("### {}", document.label);
        sections.push(heading);
        sections.push(document.content.clone());
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::memory::{DerivedMemoryKind, MemoryAuthority, MemoryRecordStatus, MemoryTrustLevel};

    #[test]
    fn collect_recent_daily_log_documents_prefers_newest_dated_logs() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(workspace_root.join("MEMORY.md"), "curated").expect("write curated memory");
        std::fs::write(memory_dir.join("2026-03-20.md"), "old").expect("write old log");
        std::fs::write(memory_dir.join("2026-03-21.md"), "middle").expect("write middle log");
        std::fs::write(memory_dir.join("2026-03-22.md"), "new").expect("write new log");

        let documents = collect_durable_recall_documents(
            workspace_root,
            256,
            "builtin",
            MemoryRecallMode::PromptAssembly,
        )
        .expect("collect durable recall documents");

        assert_eq!(documents.len(), 3);
        assert_eq!(documents[0].label, "MEMORY.md");
        assert_eq!(documents[0].provenance.scope, Some(MemoryScope::Workspace));
        assert_eq!(documents[1].label, "memory/2026-03-22.md");
        assert_eq!(documents[1].provenance.scope, Some(MemoryScope::Session));
        assert_eq!(documents[2].label, "memory/2026-03-21.md");
        assert_eq!(documents[2].provenance.scope, Some(MemoryScope::Session));
    }

    #[test]
    fn collect_curated_memory_documents_skips_empty_files() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::write(workspace_root.join("MEMORY.md"), "   ").expect("write empty memory file");
        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(memory_dir.join("2026-03-22.md"), "daily log").expect("write daily log");

        let documents = collect_durable_recall_documents(
            workspace_root,
            256,
            "builtin",
            MemoryRecallMode::PromptAssembly,
        )
        .expect("collect durable recall documents");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].label, "memory/2026-03-22.md");
    }

    #[test]
    fn load_trimmed_document_content_ignores_invalid_tail_beyond_budget() {
        let temp_dir = tempdir().expect("tempdir");
        let document_path = temp_dir.path().join("MEMORY.md");
        let mut bytes = vec![b'a'; 1600];

        bytes.push(0xff);
        bytes.push(0xfe);

        std::fs::write(&document_path, bytes).expect("write memory fixture");

        let maybe_content = load_trimmed_document_content(&document_path, 64)
            .expect("bounded durable recall read should succeed");
        let content = maybe_content.expect("content should be present");

        assert!(content.starts_with("aaaaaaaa"));
        assert!(content.contains("(truncated "));
    }

    #[test]
    fn truncate_chars_respects_budget_when_suffix_fits() {
        let input = "a".repeat(80);

        let truncated = truncate_chars(input.as_str(), 32);
        let truncated_char_count = truncated.chars().count();

        assert_eq!(truncated_char_count, 32);
        assert!(truncated.contains("(truncated "));
    }

    #[test]
    fn truncate_chars_respects_budget_when_suffix_exceeds_budget() {
        let input = "a".repeat(80);

        let truncated = truncate_chars(input.as_str(), 8);
        let truncated_char_count = truncated.chars().count();

        assert_eq!(truncated_char_count, 8);
        assert!(!truncated.is_empty());
    }

    #[test]
    fn load_durable_recall_entries_attach_source_path_and_scope_provenance() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(workspace_root.join("MEMORY.md"), "curated").expect("write curated file");
        std::fs::write(memory_dir.join("2026-03-22.md"), "daily").expect("write daily file");

        let config = MemoryRuntimeConfig::default();
        let expected_curated_path = workspace_root
            .join("MEMORY.md")
            .canonicalize()
            .expect("canonical curated path")
            .display()
            .to_string();
        let entries = load_durable_recall_entries(
            Some(workspace_root),
            &config,
            "builtin",
            MemoryRecallMode::PromptAssembly,
        )
        .expect("load durable recall entries");

        let entry = entries.first().expect("retrieved entry");
        assert_eq!(entry.provenance.len(), 2);

        let curated_provenance = &entry.provenance[0];
        assert_eq!(curated_provenance.memory_system_id, "builtin");
        assert_eq!(
            curated_provenance.source_path.as_deref(),
            Some(expected_curated_path.as_str())
        );
        assert_eq!(curated_provenance.scope, Some(MemoryScope::Workspace));
        assert_eq!(
            curated_provenance.derived_kind,
            Some(DerivedMemoryKind::Overview)
        );
        assert_eq!(
            curated_provenance.trust_level,
            Some(MemoryTrustLevel::WorkspaceCurated)
        );
        assert_eq!(
            curated_provenance.authority,
            Some(MemoryAuthority::Advisory)
        );
        assert_eq!(
            curated_provenance.record_status,
            Some(MemoryRecordStatus::Active)
        );

        let daily_provenance = &entry.provenance[1];
        assert_eq!(daily_provenance.scope, Some(MemoryScope::Session));
        assert_eq!(
            daily_provenance.recall_mode,
            MemoryRecallMode::PromptAssembly
        );
    }

    #[test]
    fn load_durable_recall_entries_skips_inactive_workspace_documents() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(
            workspace_root.join("MEMORY.md"),
            concat!("---\n", "status: tombstoned\n", "---\n", "stale note\n"),
        )
        .expect("write tombstoned file");
        std::fs::write(memory_dir.join("2026-03-22.md"), "daily").expect("write daily file");

        let config = MemoryRuntimeConfig::default();
        let entries = load_durable_recall_entries(
            Some(workspace_root),
            &config,
            "builtin",
            MemoryRecallMode::PromptAssembly,
        )
        .expect("load durable recall entries");

        let entry = entries.first().expect("retrieved entry");
        assert_eq!(entry.provenance.len(), 1);
        assert_eq!(
            entry.provenance[0].source_label.as_deref(),
            Some("memory/2026-03-22.md")
        );
    }

    #[test]
    fn workspace_document_recall_entries_honor_requested_scopes() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(workspace_root.join("MEMORY.md"), "curated").expect("write curated file");
        std::fs::write(memory_dir.join("2026-03-22.md"), "daily").expect("write daily file");

        let config = MemoryRuntimeConfig::default();
        let entries = load_workspace_document_recall_entries(
            Some(workspace_root),
            &config,
            "workspace_recall",
            MemoryRecallMode::PromptAssembly,
            &[MemoryScope::Workspace],
            4,
        )
        .expect("load workspace document recall entries");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provenance.len(), 1);
        assert_eq!(entries[0].provenance[0].scope, Some(MemoryScope::Workspace));
    }
}
