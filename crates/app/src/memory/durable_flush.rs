use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::runtime_self_continuity;

use super::runtime_config::MemoryRuntimeConfig;

const DURABLE_MEMORY_DIR: &str = "memory";
const DURABLE_MEMORY_SOURCE: &str = "pre_compaction_memory_flush";
const DURABLE_FLUSH_CLAIM_EXTENSION: &str = "claim";
const DURABLE_FLUSH_CLAIM_RETRY_DELAY: Duration = Duration::from_millis(5);
const DURABLE_FLUSH_CLAIM_RETRY_ATTEMPTS: usize = 60;

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

#[derive(Debug)]
struct DurableFlushClaimGuard {
    path: PathBuf,
}

impl Drop for DurableFlushClaimGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path.as_path());
    }
}

pub(crate) async fn flush_pre_compaction_durable_memory(
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
    let claim_guard =
        try_claim_durable_flush(target_path.as_path(), content_sha256.as_str()).await?;
    let Some(_claim_guard) = claim_guard else {
        return Ok(PreCompactionDurableFlushOutcome::SkippedDuplicate);
    };

    let is_duplicate =
        durable_flush_already_recorded(target_path.as_path(), content_sha256.as_str()).await?;
    if is_duplicate {
        return Ok(PreCompactionDurableFlushOutcome::SkippedDuplicate);
    }

    let rendered_entry = render_durable_flush_entry(
        session_id,
        summary_body.as_str(),
        exported_on.as_str(),
        content_sha256.as_str(),
    );
    append_durable_flush_entry(target_path.as_path(), rendered_entry.as_str()).await?;

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
    hex::encode(digest)
}

fn durable_memory_log_path(workspace_root: &Path, exported_on: &str) -> PathBuf {
    let file_name = format!("{exported_on}.md");
    let memory_dir = workspace_root.join(DURABLE_MEMORY_DIR);
    memory_dir.join(file_name)
}

async fn durable_flush_already_recorded(path: &Path, content_sha256: &str) -> Result<bool, String> {
    let existing = match fs::read_to_string(path).await {
        Ok(existing) => existing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "read durable memory file {} failed: {error}",
                path.display()
            ));
        }
    };
    let marker = durable_flush_hash_marker(content_sha256);

    Ok(existing.contains(marker.as_str()))
}

fn durable_flush_hash_marker(content_sha256: &str) -> String {
    format!("- content_sha256: {content_sha256}")
}

fn durable_flush_claim_path(path: &Path, _content_sha256: &str) -> Result<PathBuf, String> {
    let Some(parent) = path.parent() else {
        return Err(format!(
            "durable memory path {} has no parent directory",
            path.display()
        ));
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("durable-memory");
    let claim_file_name = format!(".{file_name}.{DURABLE_FLUSH_CLAIM_EXTENSION}");
    let claim_path = parent.join(claim_file_name);
    Ok(claim_path)
}

async fn try_claim_durable_flush(
    path: &Path,
    content_sha256: &str,
) -> Result<Option<DurableFlushClaimGuard>, String> {
    let Some(parent) = path.parent() else {
        return Err(format!(
            "durable memory path {} has no parent directory",
            path.display()
        ));
    };

    fs::create_dir_all(parent).await.map_err(|error| {
        format!(
            "create durable memory directory {} failed: {error}",
            parent.display()
        )
    })?;

    let claim_path = durable_flush_claim_path(path, content_sha256)?;
    let mut attempts_remaining = DURABLE_FLUSH_CLAIM_RETRY_ATTEMPTS;

    loop {
        let claim_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(claim_path.as_path())
            .await;

        match claim_file {
            Ok(_) => return Ok(Some(DurableFlushClaimGuard { path: claim_path })),
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempts_remaining > 0 => {
                attempts_remaining = attempts_remaining.saturating_sub(1);
                tokio::time::sleep(DURABLE_FLUSH_CLAIM_RETRY_DELAY).await;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Err(format!(
                    "timed out waiting for durable flush claim {} to clear",
                    claim_path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "create durable flush claim {} failed: {error}",
                    claim_path.display()
                ));
            }
        }
    }
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

async fn append_durable_flush_entry(path: &Path, rendered_entry: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!(
            "durable memory path {} has no parent directory",
            path.display()
        ));
    };

    fs::create_dir_all(parent).await.map_err(|error| {
        format!(
            "create durable memory directory {} failed: {error}",
            parent.display()
        )
    })?;

    let existing_len = match fs::metadata(path).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(format!(
                "read durable memory metadata {} failed: {error}",
                path.display()
            ));
        }
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|error| {
            format!(
                "open durable memory file {} failed: {error}",
                path.display()
            )
        })?;

    if existing_len > 0 {
        file.write_all(b"\n\n").await.map_err(|error| {
            format!(
                "append durable memory separator to {} failed: {error}",
                path.display()
            )
        })?;
    }

    file.write_all(rendered_entry.as_bytes())
        .await
        .map_err(|error| {
            format!(
                "append durable memory entry to {} failed: {error}",
                path.display()
            )
        })?;

    file.write_all(b"\n").await.map_err(|error| {
        format!(
            "finalize durable memory entry in {} failed: {error}",
            path.display()
        )
    })?;

    file.sync_data().await.map_err(|error| {
        format!(
            "sync durable memory file {} failed: {error}",
            path.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn durable_flush_claim_path_is_scoped_to_target_path() {
        let workspace_root = crate::test_support::unique_temp_dir("durable-flush-claim-scope");
        let target_path = workspace_root.join("memory").join("2026-03-24.md");
        let first_claim_path =
            durable_flush_claim_path(target_path.as_path(), "hash-a").expect("first claim path");
        let second_claim_path =
            durable_flush_claim_path(target_path.as_path(), "hash-b").expect("second claim path");

        assert_eq!(first_claim_path, second_claim_path);
    }

    #[tokio::test]
    async fn try_claim_durable_flush_times_out_when_claim_never_clears() {
        let workspace_root = crate::test_support::unique_temp_dir("durable-flush-claim-exists");
        let target_path = workspace_root.join("memory").join("2026-03-24.md");
        let content_sha256 = "abc123";

        let claim_path =
            durable_flush_claim_path(target_path.as_path(), content_sha256).expect("claim path");
        let parent = claim_path.parent().expect("claim parent");
        std::fs::create_dir_all(parent).expect("create claim parent");
        std::fs::write(claim_path.as_path(), "claimed").expect("write existing claim");

        let error = try_claim_durable_flush(target_path.as_path(), content_sha256)
            .await
            .expect_err("stale claim should time out");

        assert!(
            error.contains("timed out waiting for durable flush claim"),
            "stale claim should surface a timeout error"
        );
    }

    #[tokio::test]
    async fn try_claim_durable_flush_blocks_parallel_claims_for_same_target() {
        let workspace_root =
            crate::test_support::unique_temp_dir("durable-flush-claim-parallel-target");
        let target_path = workspace_root.join("memory").join("2026-03-24.md");
        let first_claim = try_claim_durable_flush(target_path.as_path(), "hash-a")
            .await
            .expect("first claim should succeed")
            .expect("first claim guard");
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired_for_thread = Arc::clone(&acquired);
        let thread_target_path = target_path.clone();

        let handle = tokio::spawn(async move {
            let second_claim = try_claim_durable_flush(thread_target_path.as_path(), "hash-b")
                .await
                .expect("second claim should eventually succeed")
                .expect("second claim guard");

            acquired_for_thread.store(true, Ordering::SeqCst);
            drop(second_claim);
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            !acquired.load(Ordering::SeqCst),
            "second claim should wait until the first target lock is released"
        );

        drop(first_claim);
        handle.await.expect("claim waiter should complete");

        assert!(
            acquired.load(Ordering::SeqCst),
            "second claim should acquire after the first target lock is released"
        );
    }
}
