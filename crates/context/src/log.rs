//! Head files that form the durable source of truth.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use loong_contracts::transcript::TranscriptItem;

/// Opens the exclusive-lock sidecar inside the store directory.
pub(super) fn open_lock_file(base: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(base.join("lock"))
}

/// Opens one head in append mode.
pub(super) fn open_head(base: &Path, generation: u64) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(head_path(base, generation))
}

/// Scans for the highest existing head generation.
///
/// Temporary `.tmp` files and the `lock` sidecar are ignored.
pub(super) fn current_gen(base: &Path) -> io::Result<Option<u64>> {
    let mut current = None;
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(digits) = name.strip_suffix(".jsonl") else {
            continue;
        };
        let Ok(generation) = digits.parse::<u64>() else {
            continue;
        };
        current = Some(current.map_or(generation, |highest: u64| highest.max(generation)));
    }
    Ok(current)
}

/// Makes a torn trailing line parseable for the next replay.
///
/// Call this only while holding the exclusive lock.
pub(super) fn repair_tail(file: &mut File) -> io::Result<()> {
    if file.metadata()?.len() == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    if last[0] != b'\n' {
        file.write_all(b"\n")?;
    }
    Ok(())
}

/// Rebuilds the working items from one head.
pub(super) fn replay_items(file: &mut File) -> io::Result<Vec<TranscriptItem>> {
    file.seek(SeekFrom::Start(0))?;
    let mut items = Vec::new();
    for line in BufReader::new(&*file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(item) = serde_json::from_str::<TranscriptItem>(&line) else {
            // Skip torn or foreign lines; the rest stays replayable.
            continue;
        };
        items.push(item);
    }
    Ok(items)
}

/// Writes a complete head to a temp file, then publishes it by rename.
///
/// A failed publish removes the temp file so the next attempt can retry.
pub(super) fn publish_head(
    base: &Path,
    generation: u64,
    items: &[TranscriptItem],
) -> io::Result<()> {
    let tmp_path = tmp_path(base, generation);
    let final_path = head_path(base, generation);
    let result = (|| {
        let mut tmp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        for item in items {
            write_item(&mut tmp, item)?;
        }
        drop(tmp);
        fs::rename(&tmp_path, &final_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Appends one item as a JSONL line.
pub(super) fn write_item(file: &mut File, item: &TranscriptItem) -> io::Result<()> {
    let mut line = serde_json::to_vec(item).expect("context items serialize");
    line.push(b'\n');
    file.write_all(&line)
}

fn head_path(base: &Path, generation: u64) -> PathBuf {
    base.join(format!("{generation}.jsonl"))
}

fn tmp_path(base: &Path, generation: u64) -> PathBuf {
    base.join(format!("{generation}.tmp"))
}
