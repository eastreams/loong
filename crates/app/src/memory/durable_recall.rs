use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::{runtime_self, runtime_self_continuity};

use super::{MemoryContextEntry, MemoryContextKind, runtime_config::MemoryRuntimeConfig};

const ROOT_MEMORY_FILE: &str = "MEMORY.md";
const NESTED_MEMORY_FILE: &str = "memory/MEMORY.md";
const RECENT_DAILY_LOG_LIMIT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableRecallDocument {
    label: String,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DailyLogCandidate {
    label: String,
    path: PathBuf,
    date: NaiveDate,
}

pub(crate) fn load_durable_recall_entries(
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<MemoryContextEntry>, String> {
    let Some(workspace_root) = workspace_root else {
        return Ok(Vec::new());
    };

    let canonical_workspace_root = canonical_workspace_memory_root(workspace_root)?;
    let candidate_roots = runtime_self::candidate_workspace_roots(workspace_root);
    let per_file_char_budget = config.summary_max_chars.max(256);

    let curated_documents = collect_curated_memory_documents(
        workspace_root,
        canonical_workspace_root.as_path(),
        candidate_roots.as_slice(),
        per_file_char_budget,
    )?;
    let daily_documents = collect_recent_daily_log_documents(
        workspace_root,
        canonical_workspace_root.as_path(),
        candidate_roots.as_slice(),
        per_file_char_budget,
    )?;

    let mut documents = Vec::new();
    documents.extend(curated_documents);
    documents.extend(daily_documents);

    if documents.is_empty() {
        return Ok(Vec::new());
    }

    let content = render_durable_recall_block(documents.as_slice());
    let entry = MemoryContextEntry {
        kind: MemoryContextKind::RetrievedMemory,
        role: "system".to_owned(),
        content,
    };

    Ok(vec![entry])
}

fn collect_curated_memory_documents(
    workspace_root: &Path,
    canonical_workspace_root: &Path,
    candidate_roots: &[PathBuf],
    per_file_char_budget: usize,
) -> Result<Vec<DurableRecallDocument>, String> {
    let mut documents = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let relative_paths = [ROOT_MEMORY_FILE, NESTED_MEMORY_FILE];

    for root in candidate_roots {
        for relative_path in relative_paths {
            let candidate_path = root.join(relative_path);
            let maybe_document = load_document_if_present(
                workspace_root,
                canonical_workspace_root,
                candidate_path.as_path(),
                per_file_char_budget,
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

fn collect_recent_daily_log_documents(
    workspace_root: &Path,
    canonical_workspace_root: &Path,
    candidate_roots: &[PathBuf],
    per_file_char_budget: usize,
) -> Result<Vec<DurableRecallDocument>, String> {
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
                "read durable recall directory {} failed: {error}",
                memory_dir.display()
            )
        })?;
        for entry_result in read_dir {
            let entry = entry_result.map_err(|error| {
                format!(
                    "read durable recall directory entry in {} failed: {error}",
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

            let label = durable_recall_label(workspace_root, path.as_path());
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
    let selected_candidates = candidates.into_iter().take(RECENT_DAILY_LOG_LIMIT);
    for candidate in selected_candidates {
        let maybe_content =
            load_trimmed_document_content(candidate.path.as_path(), per_file_char_budget)?;
        let Some(content) = maybe_content else {
            continue;
        };
        let document = DurableRecallDocument {
            label: candidate.label,
            content,
        };
        documents.push(document);
    }

    Ok(documents)
}

fn load_document_if_present(
    workspace_root: &Path,
    canonical_workspace_root: &Path,
    path: &Path,
    per_file_char_budget: usize,
    seen_paths: &mut BTreeSet<String>,
) -> Result<Option<DurableRecallDocument>, String> {
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

    let maybe_content =
        load_trimmed_document_content(canonical_path.as_path(), per_file_char_budget)?;
    let Some(content) = maybe_content else {
        return Ok(None);
    };

    let label = durable_recall_label(workspace_root, path);
    let document = DurableRecallDocument { label, content };
    Ok(Some(document))
}

fn load_trimmed_document_content(
    path: &Path,
    per_file_char_budget: usize,
) -> Result<Option<String>, String> {
    let raw_content = std::fs::read_to_string(path).map_err(|error| {
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

fn parse_daily_log_date(path: &Path) -> Option<NaiveDate> {
    let extension = path.extension().and_then(|value| value.to_str());
    let is_markdown = extension.is_some_and(|value| value.eq_ignore_ascii_case("md"));
    if !is_markdown {
        return None;
    }

    let stem = path.file_stem().and_then(|value| value.to_str())?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

fn durable_recall_label(workspace_root: &Path, path: &Path) -> String {
    let relative_path = path.strip_prefix(workspace_root).ok();
    let display_path = relative_path.unwrap_or(path);
    display_path.display().to_string()
}

fn canonical_workspace_memory_root(workspace_root: &Path) -> Result<PathBuf, String> {
    workspace_root.canonicalize().map_err(|error| {
        format!(
            "canonicalize durable recall workspace root {} failed: {error}",
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
            "canonicalize durable recall path {} failed: {error}",
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

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_owned();
    }

    let mut truncated = String::new();
    let kept_chars = max_chars.saturating_sub(1);
    for ch in input.chars().take(kept_chars) {
        truncated.push(ch);
    }

    let removed_chars = char_count.saturating_sub(kept_chars);
    truncated.push_str(&format!("...(truncated {removed_chars} chars)"));
    truncated
}

fn render_durable_recall_block(documents: &[DurableRecallDocument]) -> String {
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
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[test]
    fn collect_recent_daily_log_documents_prefers_newest_dated_logs() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let memory_dir = workspace_root.join("memory");
        std::fs::create_dir_all(&memory_dir).expect("create memory dir");

        std::fs::write(memory_dir.join("2026-03-20.md"), "old").expect("write old log");
        std::fs::write(memory_dir.join("2026-03-21.md"), "middle").expect("write middle log");
        std::fs::write(memory_dir.join("2026-03-22.md"), "new").expect("write new log");

        let candidate_roots = runtime_self::candidate_workspace_roots(workspace_root);
        let canonical_workspace_root =
            canonical_workspace_memory_root(workspace_root).expect("canonical workspace root");
        let documents = collect_recent_daily_log_documents(
            workspace_root,
            canonical_workspace_root.as_path(),
            candidate_roots.as_slice(),
            256,
        )
        .expect("collect daily log documents");

        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].label, "memory/2026-03-22.md");
        assert_eq!(documents[1].label, "memory/2026-03-21.md");
    }

    #[test]
    fn collect_curated_memory_documents_skips_empty_files() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();

        std::fs::write(workspace_root.join("MEMORY.md"), "   ").expect("write empty memory file");

        let candidate_roots = runtime_self::candidate_workspace_roots(workspace_root);
        let canonical_workspace_root =
            canonical_workspace_memory_root(workspace_root).expect("canonical workspace root");
        let documents = collect_curated_memory_documents(
            workspace_root,
            canonical_workspace_root.as_path(),
            candidate_roots.as_slice(),
            256,
        )
        .expect("collect curated memory documents");

        assert!(documents.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn load_durable_recall_entries_ignores_symlinked_memory_file_outside_workspace_root() {
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

        let config = MemoryRuntimeConfig::default();
        let entries = load_durable_recall_entries(Some(workspace_root.as_path()), &config)
            .expect("load durable recall entries");

        assert!(entries.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn load_durable_recall_entries_ignores_symlinked_memory_directory_outside_workspace_root() {
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

        let config = MemoryRuntimeConfig::default();
        let entries = load_durable_recall_entries(Some(workspace_root.as_path()), &config)
            .expect("load durable recall entries");

        assert!(entries.is_empty());
    }
}
