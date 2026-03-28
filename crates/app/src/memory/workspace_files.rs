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
    let canonical_workspace_root = canonical_workspace_memory_root(workspace_root)?;
    let candidate_roots = runtime_self::candidate_workspace_roots(workspace_root);
    let curated_documents = collect_curated_memory_document_locations(
        workspace_root,
        canonical_workspace_root.as_path(),
        candidate_roots.as_slice(),
    )?;
    let daily_documents = collect_daily_log_document_locations(
        workspace_root,
        canonical_workspace_root.as_path(),
        candidate_roots.as_slice(),
    )?;

    let mut documents = Vec::new();
    documents.extend(curated_documents);
    documents.extend(daily_documents);

    Ok(documents)
}

fn collect_curated_memory_document_locations(
    workspace_root: &Path,
    canonical_workspace_root: &Path,
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
                canonical_workspace_root,
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
    canonical_workspace_root: &Path,
    candidate_roots: &[PathBuf],
) -> Result<Vec<WorkspaceMemoryDocumentLocation>, String> {
    let mut candidates = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for root in candidate_roots {
        let memory_dir = root.join("memory");
        if !memory_dir.is_dir() {
            continue;
        }

        let Some(_canonical_memory_dir) =
            resolve_workspace_memory_path(canonical_workspace_root, memory_dir.as_path())?
        else {
            continue;
        };

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

            let Some(canonical_path) =
                resolve_workspace_memory_path(canonical_workspace_root, path.as_path())?
            else {
                continue;
            };

            let path_key = canonical_path_key(canonical_path.as_path());
            let inserted = seen_paths.insert(path_key);
            if !inserted {
                continue;
            }

            let label = workspace_memory_label(workspace_root, path.as_path());
            let candidate = DailyLogCandidate {
                label,
                path: canonical_path,
                date,
            };
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
    canonical_workspace_root: &Path,
    path: &Path,
    kind: WorkspaceMemoryDocumentKind,
    date: Option<NaiveDate>,
    seen_paths: &mut BTreeSet<String>,
) -> Result<Option<WorkspaceMemoryDocumentLocation>, String> {
    if !path.is_file() {
        return Ok(None);
    }

    let Some(canonical_path) = resolve_workspace_memory_path(canonical_workspace_root, path)?
    else {
        return Ok(None);
    };

    let path_key = canonical_path_key(canonical_path.as_path());
    let inserted = seen_paths.insert(path_key);
    if !inserted {
        return Ok(None);
    }

    let label = workspace_memory_label(workspace_root, path);
    let document = WorkspaceMemoryDocumentLocation {
        label,
        path: canonical_path,
        kind,
        date,
    };

    Ok(Some(document))
}

pub(crate) fn workspace_memory_label(workspace_root: &Path, path: &Path) -> String {
    let relative_path = path.strip_prefix(workspace_root).ok();
    let display_path = relative_path.unwrap_or(path);
    let raw_label = display_path.display().to_string();

    if cfg!(windows) {
        let normalized_label = raw_label.replace('\\', "/");
        return normalized_label;
    }

    raw_label
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

fn canonical_workspace_memory_root(workspace_root: &Path) -> Result<PathBuf, String> {
    workspace_root.canonicalize().map_err(|error| {
        format!(
            "canonicalize workspace memory root {} failed: {error}",
            workspace_root.display()
        )
    })
}

fn resolve_workspace_memory_path(
    canonical_workspace_root: &Path,
    path: &Path,
) -> Result<Option<PathBuf>, String> {
    let canonical_path = path.canonicalize().map_err(|error| {
        format!(
            "canonicalize workspace memory path {} failed: {error}",
            path.display()
        )
    })?;
    let is_within_workspace = canonical_path.starts_with(canonical_workspace_root);
    if !is_within_workspace {
        return Ok(None);
    }

    Ok(Some(canonical_path))
}

fn canonical_path_key(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

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

    #[cfg(unix)]
    #[test]
    fn workspace_memory_label_preserves_backslashes_in_unix_file_names() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");
        let file_name = "2026-03-26\\notes.md";
        let daily_log_path = memory_dir.join(file_name);

        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(&daily_log_path, "notes").expect("write daily log");

        let label = workspace_memory_label(workspace_root, daily_log_path.as_path());

        assert_eq!(label, format!("memory/{file_name}"));
    }

    #[cfg(unix)]
    #[test]
    fn collect_workspace_memory_document_locations_ignores_symlinked_memory_file_outside_workspace_root()
     {
        let temp_dir = tempdir().expect("tempdir");
        let sandbox_root = temp_dir.path();
        let workspace_root = sandbox_root.join("workspace");
        let outside_root = sandbox_root.join("outside");
        let outside_memory_path = outside_root.join("secret.md");
        let linked_memory_path = workspace_root.join("MEMORY.md");

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_root).expect("create outside root");
        std::fs::write(&outside_memory_path, "outside durable recall secret")
            .expect("write outside memory");
        create_symlink(&outside_memory_path, &linked_memory_path).expect("create symlink");

        let documents = collect_workspace_memory_document_locations(workspace_root.as_path())
            .expect("collect workspace memory documents");

        assert!(documents.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn collect_workspace_memory_document_locations_ignores_symlinked_memory_directory_outside_workspace_root()
     {
        let temp_dir = tempdir().expect("tempdir");
        let sandbox_root = temp_dir.path();
        let workspace_root = sandbox_root.join("workspace");
        let outside_root = sandbox_root.join("outside");
        let outside_memory_dir = outside_root.join("logs");
        let outside_daily_log_path = outside_memory_dir.join("2026-03-24.md");
        let linked_memory_dir = workspace_root.join("memory");

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_memory_dir).expect("create outside memory dir");
        std::fs::write(&outside_daily_log_path, "outside durable recall daily log")
            .expect("write outside daily log");
        create_symlink(&outside_memory_dir, &linked_memory_dir).expect("create dir symlink");

        let documents = collect_workspace_memory_document_locations(workspace_root.as_path())
            .expect("collect workspace memory documents");

        assert!(documents.is_empty());
    }
}
