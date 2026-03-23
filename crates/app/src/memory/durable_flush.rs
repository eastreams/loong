use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;
use sha2::{Digest, Sha256};

use crate::runtime_self_continuity;

use super::runtime_config::MemoryRuntimeConfig;

const DURABLE_MEMORY_DIR: &str = "memory";
const DURABLE_MEMORY_SOURCE: &str = "pre_compaction_memory_flush";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreCompactionDurableFlushOutcome {
    SkippedMissingWorkspaceRoot,
    SkippedNoSummary,
    SkippedDuplicate,
    Flushed {
        path: PathBuf,
        content_sha256: String,
    },
}

pub(crate) fn flush_pre_compaction_durable_memory(
    session_id: &str,
    workspace_root: Option<&Path>,
    memory_config: &MemoryRuntimeConfig,
) -> Result<PreCompactionDurableFlushOutcome, String> {
    let Some(workspace_root) = workspace_root else {
        return Ok(PreCompactionDurableFlushOutcome::SkippedMissingWorkspaceRoot);
    };

    let summary_body =
        super::sqlite::load_summary_body_for_durable_flush(session_id, memory_config)?;
    let Some(summary_body) = summary_body else {
        return Ok(PreCompactionDurableFlushOutcome::SkippedNoSummary);
    };

    let exported_on = Local::now().format("%Y-%m-%d").to_string();
    let content_sha256 = durable_flush_content_sha256(session_id, summary_body.as_str());
    let target_path = durable_memory_log_path(workspace_root, exported_on.as_str());

    let is_duplicate =
        durable_flush_already_recorded(target_path.as_path(), content_sha256.as_str())?;
    if is_duplicate {
        return Ok(PreCompactionDurableFlushOutcome::SkippedDuplicate);
    }

    let rendered_entry = render_durable_flush_entry(
        session_id,
        summary_body.as_str(),
        exported_on.as_str(),
        content_sha256.as_str(),
    );
    append_durable_flush_entry(target_path.as_path(), rendered_entry.as_str())?;

    Ok(PreCompactionDurableFlushOutcome::Flushed {
        path: target_path,
        content_sha256,
    })
}

fn durable_flush_content_sha256(session_id: &str, summary_body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(summary_body.as_bytes());

    let digest = hasher.finalize();
    format!("{digest:x}")
}

fn durable_memory_log_path(workspace_root: &Path, exported_on: &str) -> PathBuf {
    let file_name = format!("{exported_on}.md");
    let memory_dir = workspace_root.join(DURABLE_MEMORY_DIR);
    memory_dir.join(file_name)
}

fn durable_flush_already_recorded(path: &Path, content_sha256: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let existing = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "read durable memory file {} failed: {error}",
            path.display()
        )
    })?;
    let marker = durable_flush_hash_marker(content_sha256);

    Ok(existing.contains(marker.as_str()))
}

fn durable_flush_hash_marker(content_sha256: &str) -> String {
    format!("- content_sha256: {content_sha256}")
}

fn render_durable_flush_entry(
    session_id: &str,
    summary_body: &str,
    exported_on: &str,
    content_sha256: &str,
) -> String {
    let intro = runtime_self_continuity::durable_recall_intro();
    let hash_marker = durable_flush_hash_marker(content_sha256);

    let sections = [
        "## Durable Recall".to_owned(),
        intro.to_owned(),
        format!("- source: {DURABLE_MEMORY_SOURCE}"),
        format!("- session_id: {session_id}"),
        format!("- exported_on: {exported_on}"),
        hash_marker,
        summary_body.trim().to_owned(),
    ];
    sections.join("\n\n")
}

fn append_durable_flush_entry(path: &Path, rendered_entry: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!(
            "durable memory path {} has no parent directory",
            path.display()
        ));
    };

    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create durable memory directory {} failed: {error}",
            parent.display()
        )
    })?;

    let existing_len = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "open durable memory file {} failed: {error}",
                path.display()
            )
        })?;

    if existing_len > 0 {
        file.write_all(b"\n\n").map_err(|error| {
            format!(
                "append durable memory separator to {} failed: {error}",
                path.display()
            )
        })?;
    }

    file.write_all(rendered_entry.as_bytes()).map_err(|error| {
        format!(
            "append durable memory entry to {} failed: {error}",
            path.display()
        )
    })?;

    file.write_all(b"\n").map_err(|error| {
        format!(
            "finalize durable memory entry in {} failed: {error}",
            path.display()
        )
    })?;

    Ok(())
}
