use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::runtime_self;

pub(crate) const ROOT_MEMORY_FILE: &str = "MEMORY.md";
pub(crate) const NESTED_MEMORY_FILE: &str = "memory/MEMORY.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceMemoryDocumentKind {
    Curated,
    DailyLog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceMemoryDocumentLocation {
    pub label: String,
    pub path: PathBuf,
    pub kind: WorkspaceMemoryDocumentKind,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DailyLogCandidate {
    label: String,
    path: PathBuf,
    date: NaiveDate,
}

pub(crate) fn collect_workspace_memory_document_locations(
    workspace_root: &Path,
) -> Result<Vec<WorkspaceMemoryDocumentLocation>, String> {
    let candidate_roots = runtime_self::candidate_workspace_roots(workspace_root);
    let curated_documents =
        collect_curated_memory_document_locations(workspace_root, candidate_roots.as_slice())?;
    let daily_documents =
        collect_daily_log_document_locations(workspace_root, candidate_roots.as_slice())?;

    let mut documents = Vec::new();
    documents.extend(curated_documents);
    documents.extend(daily_documents);

    Ok(documents)
}

fn collect_curated_memory_document_locations(
    workspace_root: &Path,
    candidate_roots: &[PathBuf],
) -> Result<Vec<WorkspaceMemoryDocumentLocation>, String> {
    let mut documents = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let relative_paths = [ROOT_MEMORY_FILE, NESTED_MEMORY_FILE];

    for root in candidate_roots {
        for relative_path in relative_paths {
            let candidate_path = root.join(relative_path);
            let maybe_document = collect_document_if_present(
                workspace_root,
                candidate_path.as_path(),
                WorkspaceMemoryDocumentKind::Curated,
                None,
                &mut seen_paths,
            )?;
            let Some(document) = maybe_document else {
                continue;
            };
            documents.push(document);
        }
    }

    Ok(documents)
}

fn collect_daily_log_document_locations(
    workspace_root: &Path,
    candidate_roots: &[PathBuf],
) -> Result<Vec<WorkspaceMemoryDocumentLocation>, String> {
    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for root in candidate_roots {
        let memory_dir = root.join("memory");
        if !memory_dir.is_dir() {
            continue;
        }

        let read_dir = std::fs::read_dir(&memory_dir).map_err(|error| {
            format!(
                "read workspace memory directory {} failed: {error}",
                memory_dir.display()
            )
        })?;
        for entry_result in read_dir {
            let entry = entry_result.map_err(|error| {
                format!(
                    "read workspace memory directory entry in {} failed: {error}",
                    memory_dir.display()
                )
            })?;
            let path = entry.path();
            let Some(date) = parse_daily_log_date(path.as_path()) else {
                continue;
            };

            let path_key = normalized_path_key(path.as_path());
            let inserted = seen_paths.insert(path_key);
            if !inserted {
                continue;
            }

            let label = workspace_memory_label(workspace_root, path.as_path());
            let candidate = DailyLogCandidate { label, path, date };
            candidates.push(candidate);
        }
    }

    candidates.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then(left.label.cmp(&right.label))
    });

    let mut documents = Vec::new();
    for candidate in candidates {
        let document = WorkspaceMemoryDocumentLocation {
            label: candidate.label,
            path: candidate.path,
            kind: WorkspaceMemoryDocumentKind::DailyLog,
            date: Some(candidate.date),
        };
        documents.push(document);
    }

    Ok(documents)
}

fn collect_document_if_present(
    workspace_root: &Path,
    path: &Path,
    kind: WorkspaceMemoryDocumentKind,
    date: Option<NaiveDate>,
    seen_paths: &mut BTreeSet<String>,
) -> Result<Option<WorkspaceMemoryDocumentLocation>, String> {
    if !path.is_file() {
        return Ok(None);
    }

    let path_key = normalized_path_key(path);
    let inserted = seen_paths.insert(path_key);
    if !inserted {
        return Ok(None);
    }

    let label = workspace_memory_label(workspace_root, path);
    let document = WorkspaceMemoryDocumentLocation {
        label,
        path: path.to_path_buf(),
        kind,
        date,
    };

    Ok(Some(document))
}

pub(crate) fn workspace_memory_label(workspace_root: &Path, path: &Path) -> String {
    let relative_path = path.strip_prefix(workspace_root).ok();
    let display_path = relative_path.unwrap_or(path);
    display_path.display().to_string()
}

fn parse_daily_log_date(path: &Path) -> Option<NaiveDate> {
    let extension = path.extension().and_then(|value| value.to_str());
    let is_markdown = extension.is_some_and(|value| value.eq_ignore_ascii_case("md"));
    if !is_markdown {
        return None;
    }

    let stem = path.file_stem().and_then(|value| value.to_str())?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

fn normalized_path_key(path: &Path) -> String {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn collect_workspace_memory_document_locations_prefers_newest_daily_logs_first() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(workspace_root.join("MEMORY.md"), "root").expect("write root memory");
        std::fs::write(memory_dir.join("2026-03-20.md"), "old").expect("write old log");
        std::fs::write(memory_dir.join("2026-03-22.md"), "new").expect("write new log");

        let documents = collect_workspace_memory_document_locations(workspace_root)
            .expect("collect workspace memory documents");

        assert_eq!(documents.len(), 3);
        assert_eq!(documents[0].label, "MEMORY.md");
        assert_eq!(documents[1].label, "memory/2026-03-22.md");
        assert_eq!(documents[2].label, "memory/2026-03-20.md");
    }

    #[test]
    fn collect_workspace_memory_document_locations_includes_nested_workspace_memory() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace");
        std::fs::write(nested_workspace_root.join("MEMORY.md"), "nested memory")
            .expect("write nested memory");

        let documents = collect_workspace_memory_document_locations(workspace_root)
            .expect("collect workspace memory documents");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].label, "workspace/MEMORY.md");
    }
}
