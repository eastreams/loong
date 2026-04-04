use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use rand::random;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Re-export data types from contracts
pub use loongclaw_contracts::{AuditEvent, AuditEventKind, ExecutionPlane, PlaneTier};

use crate::errors::AuditError;

type HmacSha256 = Hmac<Sha256>;

pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError>;
}

#[derive(Debug, Default)]
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn record(&self, _event: AuditEvent) -> Result<(), AuditError> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditSink {
    #[must_use]
    pub fn snapshot(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .map_or_else(|_| Vec::new(), |guard| guard.clone())
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let mut guard = self
            .events
            .lock()
            .map_err(|_err| AuditError::Sink("audit mutex poisoned".to_owned()))?;
        guard.push(event);
        Ok(())
    }
}

#[derive(Debug)]
pub struct JsonlAuditSink {
    path: PathBuf,
    journal: Mutex<File>,
    integrity_paths: AuditIntegrityPaths,
    integrity_key: Vec<u8>,
    integrity_journal: Mutex<File>,
    integrity_seal: Mutex<File>,
    integrity_state: Mutex<AuditIntegrityState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditIntegrityPaths {
    pub integrity_journal_path: PathBuf,
    pub key_path: PathBuf,
    pub seal_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditJournalIntegrityReport {
    pub journal_path: PathBuf,
    pub paths: AuditIntegrityPaths,
    pub protected_entries: usize,
    pub last_event_id: Option<String>,
    pub status: AuditJournalIntegrityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditJournalIntegrityRepairAction {
    NoopVerified,
    RepairedMissingArtifacts,
    RefusedMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditJournalIntegrityRepairReport {
    pub action: AuditJournalIntegrityRepairAction,
    pub report: AuditJournalIntegrityReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuditJournalIntegrityStatus {
    Verified,
    MissingArtifacts { missing_paths: Vec<String> },
    Mismatch { line: Option<usize>, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AuditIntegrityState {
    entry_count: usize,
    journal_bytes: u64,
    integrity_bytes: u64,
    seal_bytes: u64,
    last_event_id: Option<String>,
    last_chain_hmac: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuditIntegrityVerificationOutcome {
    Verified(AuditIntegrityState),
    Mismatch {
        protected_entries: usize,
        last_event_id: Option<String>,
        line: Option<usize>,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuditIntegrityRecord {
    event_id: String,
    line_sha256_hex: String,
    chain_hmac_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuditIntegritySeal {
    entry_count: usize,
    journal_bytes: u64,
    integrity_bytes: u64,
    last_event_id: Option<String>,
    last_chain_hmac_hex: String,
    seal_hmac_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AuditIntegritySealUnsigned<'a> {
    entry_count: usize,
    journal_bytes: u64,
    integrity_bytes: u64,
    last_event_id: Option<&'a str>,
    last_chain_hmac_hex: &'a str,
}

struct AuditFileLock {
    file: Option<File>,
    path: PathBuf,
}

impl AuditFileLock {
    fn new(file: File, path: &Path) -> Result<Self, AuditError> {
        lock_audit_file(&file, path)?;
        Ok(Self {
            file: Some(file),
            path: path.to_path_buf(),
        })
    }

    fn into_file(mut self) -> Result<File, AuditError> {
        let Some(file) = self.file.take() else {
            return Err(AuditError::Sink(
                "audit file lock should still hold a file".to_owned(),
            ));
        };

        Ok(file)
    }
}

impl Drop for AuditFileLock {
    fn drop(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let _ = unlock_audit_file(file, &self.path);
    }
}

fn prepare_audit_journal_parent(path: &Path) -> Result<(), AuditError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            AuditError::Sink(format!(
                "failed to prepare audit journal parent directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }

    Ok(())
}

fn open_jsonl_audit_journal(path: &Path) -> Result<File, AuditError> {
    prepare_audit_journal_parent(path)?;

    OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            AuditError::Sink(format!(
                "failed to open audit journal `{}`: {error}",
                path.display()
            ))
        })
}

fn open_read_write_file(path: &Path) -> Result<File, AuditError> {
    prepare_audit_journal_parent(path)?;

    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            AuditError::Sink(format!(
                "failed to open sidecar file `{}`: {error}",
                path.display()
            ))
        })
}

fn open_existing_read_append_file(path: &Path) -> Result<File, AuditError> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            AuditError::Sink(format!(
                "failed to open existing audit file `{}`: {error}",
                path.display()
            ))
        })
}

fn lock_audit_file(file: &File, path: &Path) -> Result<(), AuditError> {
    file.lock().map_err(|error| {
        AuditError::Sink(format!(
            "failed to lock audit file `{}`: {error}",
            path.display()
        ))
    })
}

fn unlock_audit_file(file: &File, path: &Path) -> Result<(), AuditError> {
    file.unlock().map_err(|error| {
        AuditError::Sink(format!(
            "failed to unlock audit file `{}`: {error}",
            path.display()
        ))
    })
}

/// Exercise the same open + lock + unlock path that production audit writes use.
pub fn probe_jsonl_audit_journal_runtime_ready(path: &Path) -> Result<(), AuditError> {
    let journal = open_jsonl_audit_journal(path)?;
    lock_audit_file(&journal, path)?;
    unlock_audit_file(&journal, path)
}

impl JsonlAuditSink {
    pub fn new(path: PathBuf) -> Result<Self, AuditError> {
        let integrity_paths = derive_jsonl_audit_integrity_paths(&path);
        let journal_lock = AuditFileLock::new(open_jsonl_audit_journal(&path)?, &path)?;
        let integrity_key = ensure_audit_integrity_key(&integrity_paths)?;

        initialize_audit_integrity_state(&path, &integrity_paths, &integrity_key)?;
        let integrity_journal_lock = AuditFileLock::new(
            open_existing_read_append_file(&integrity_paths.integrity_journal_path)?,
            &integrity_paths.integrity_journal_path,
        )?;
        let integrity_seal_lock = AuditFileLock::new(
            open_existing_read_write_file(&integrity_paths.seal_path, "audit integrity seal")?,
            &integrity_paths.seal_path,
        )?;
        let integrity_state = load_audit_integrity_state(&path, &integrity_paths, &integrity_key)?;
        let journal = journal_lock.into_file()?;
        let integrity_journal = integrity_journal_lock.into_file()?;
        let integrity_seal = integrity_seal_lock.into_file()?;

        Ok(Self {
            path,
            journal: Mutex::new(journal),
            integrity_paths,
            integrity_key,
            integrity_journal: Mutex::new(integrity_journal),
            integrity_seal: Mutex::new(integrity_seal),
            integrity_state: Mutex::new(integrity_state),
        })
    }
}

pub fn derive_jsonl_audit_integrity_paths(journal_path: &Path) -> AuditIntegrityPaths {
    let parent = journal_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = journal_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("events.jsonl");
    let base_name = file_name.strip_suffix(".jsonl").unwrap_or(file_name);

    let integrity_journal_path = parent.join(format!("{base_name}.integrity.jsonl"));
    let key_path = parent.join(format!("{base_name}.integrity.key"));
    let seal_path = parent.join(format!("{base_name}.integrity.seal.json"));

    AuditIntegrityPaths {
        integrity_journal_path,
        key_path,
        seal_path,
    }
}

pub fn verify_jsonl_audit_journal_integrity(
    journal_path: &Path,
) -> Result<AuditJournalIntegrityReport, AuditError> {
    validate_existing_audit_journal(journal_path)?;

    let journal = open_existing_read_write_file(journal_path, "audit journal")?;
    lock_audit_file(&journal, journal_path)?;

    let integrity_paths = derive_jsonl_audit_integrity_paths(journal_path);
    let report_result = verify_jsonl_audit_journal_integrity_locked(journal_path, integrity_paths);
    let unlock_result = unlock_audit_file(&journal, journal_path);

    match (report_result, unlock_result) {
        (Err(error), _) => Err(error),
        (Ok(_report), Err(error)) => Err(error),
        (Ok(report), Ok(())) => Ok(report),
    }
}

pub fn repair_jsonl_audit_journal_integrity(
    journal_path: &Path,
) -> Result<AuditJournalIntegrityRepairReport, AuditError> {
    validate_existing_audit_journal(journal_path)?;

    let journal = open_existing_read_write_file(journal_path, "audit journal")?;
    lock_audit_file(&journal, journal_path)?;

    let integrity_paths = derive_jsonl_audit_integrity_paths(journal_path);
    let report_before =
        verify_jsonl_audit_journal_integrity_locked(journal_path, integrity_paths.clone())?;

    let action = match &report_before.status {
        AuditJournalIntegrityStatus::Verified => AuditJournalIntegrityRepairAction::NoopVerified,
        AuditJournalIntegrityStatus::MissingArtifacts { .. } => {
            let integrity_key = ensure_audit_integrity_key(&integrity_paths)?;
            let integrity_journal =
                open_jsonl_audit_journal(&integrity_paths.integrity_journal_path)?;
            lock_audit_file(&integrity_journal, &integrity_paths.integrity_journal_path)?;
            let seal = open_read_write_file(&integrity_paths.seal_path)?;
            lock_audit_file(&seal, &integrity_paths.seal_path)?;

            let rebuild_result =
                rebuild_audit_integrity_state(journal_path, &integrity_paths, &integrity_key);
            let unlock_seal_result = unlock_audit_file(&seal, &integrity_paths.seal_path);
            let unlock_integrity_result =
                unlock_audit_file(&integrity_journal, &integrity_paths.integrity_journal_path);

            match (rebuild_result, unlock_seal_result, unlock_integrity_result) {
                (Err(error), _, _) => return Err(error),
                (Ok(()), Err(error), _) => return Err(error),
                (Ok(()), Ok(()), Err(error)) => return Err(error),
                (Ok(()), Ok(()), Ok(())) => {
                    AuditJournalIntegrityRepairAction::RepairedMissingArtifacts
                }
            }
        }
        AuditJournalIntegrityStatus::Mismatch { .. } => {
            AuditJournalIntegrityRepairAction::RefusedMismatch
        }
    };

    let report_result = match action {
        AuditJournalIntegrityRepairAction::NoopVerified => Ok(report_before),
        AuditJournalIntegrityRepairAction::RepairedMissingArtifacts => {
            verify_jsonl_audit_journal_integrity_locked(journal_path, integrity_paths)
        }
        AuditJournalIntegrityRepairAction::RefusedMismatch => Ok(report_before),
    };
    let unlock_result = unlock_audit_file(&journal, journal_path);

    match (report_result, unlock_result) {
        (Err(error), _) => Err(error),
        (Ok(_report), Err(error)) => Err(error),
        (Ok(report), Ok(())) => Ok(AuditJournalIntegrityRepairReport { action, report }),
    }
}

fn serialize_audit_event_line(
    event: &AuditEvent,
    journal_path: &Path,
) -> Result<Vec<u8>, AuditError> {
    let mut encoded = serde_json::to_vec(event).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit event for `{}`: {error}",
            journal_path.display()
        ))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn serialize_audit_integrity_record_line(
    record: &AuditIntegrityRecord,
    path: &Path,
) -> Result<Vec<u8>, AuditError> {
    let mut encoded = serde_json::to_vec(record).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit integrity record for `{}`: {error}",
            path.display()
        ))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn serialize_audit_integrity_seal(
    seal: &AuditIntegritySeal,
    path: &Path,
) -> Result<Vec<u8>, AuditError> {
    let mut encoded = serde_json::to_vec(seal).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit integrity seal for `{}`: {error}",
            path.display()
        ))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn ensure_audit_integrity_key(paths: &AuditIntegrityPaths) -> Result<Vec<u8>, AuditError> {
    if paths.key_path.exists() {
        return read_audit_integrity_key(&paths.key_path);
    }

    let key_bytes = random::<[u8; 32]>().to_vec();
    write_new_audit_integrity_key(&paths.key_path, &key_bytes)
}

fn read_audit_integrity_key(path: &Path) -> Result<Vec<u8>, AuditError> {
    let key_bytes = fs::read(path).map_err(|error| {
        AuditError::Sink(format!(
            "failed to read audit integrity key `{}`: {error}",
            path.display()
        ))
    })?;

    if key_bytes.len() != 32 {
        return Err(AuditError::Sink(format!(
            "audit integrity key `{}` must contain exactly 32 bytes, found {}",
            path.display(),
            key_bytes.len()
        )));
    }

    Ok(key_bytes)
}

fn write_new_audit_integrity_key(path: &Path, key_bytes: &[u8]) -> Result<Vec<u8>, AuditError> {
    prepare_audit_journal_parent(path)?;

    let open_result = OpenOptions::new().create_new(true).write(true).open(path);

    let mut file = match open_result {
        Ok(file) => file,
        Err(error) => {
            if error.kind() == ErrorKind::AlreadyExists {
                return read_audit_integrity_key(path);
            }
            return Err(AuditError::Sink(format!(
                "failed to create audit integrity key `{}`: {error}",
                path.display()
            )));
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            AuditError::Sink(format!(
                "failed to set permissions on audit integrity key `{}`: {error}",
                path.display()
            ))
        })?;
    }

    file.write_all(key_bytes).map_err(|error| {
        AuditError::Sink(format!(
            "failed to write audit integrity key `{}`: {error}",
            path.display()
        ))
    })?;
    file.flush().map_err(|error| {
        AuditError::Sink(format!(
            "failed to flush audit integrity key `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(key_bytes.to_vec())
}

fn verify_jsonl_audit_journal_integrity_locked(
    journal_path: &Path,
    integrity_paths: AuditIntegrityPaths,
) -> Result<AuditJournalIntegrityReport, AuditError> {
    let missing_paths = collect_missing_integrity_paths(&integrity_paths);
    let all_integrity_artifacts_missing = missing_paths.len() == 3;

    if !missing_paths.is_empty() {
        if !all_integrity_artifacts_missing {
            let reason = format!(
                "audit integrity sidecar is partially missing: {}",
                missing_paths.join(", ")
            );
            let report = AuditJournalIntegrityReport {
                journal_path: journal_path.to_path_buf(),
                paths: integrity_paths,
                protected_entries: 0,
                last_event_id: None,
                status: AuditJournalIntegrityStatus::Mismatch { line: None, reason },
            };
            return Ok(report);
        }

        let report = AuditJournalIntegrityReport {
            journal_path: journal_path.to_path_buf(),
            paths: integrity_paths,
            protected_entries: 0,
            last_event_id: None,
            status: AuditJournalIntegrityStatus::MissingArtifacts { missing_paths },
        };
        return Ok(report);
    }

    let integrity_journal =
        open_existing_read_append_file(&integrity_paths.integrity_journal_path)?;
    lock_audit_file(&integrity_journal, &integrity_paths.integrity_journal_path)?;
    let seal = open_existing_read_write_file(&integrity_paths.seal_path, "audit integrity seal")?;
    lock_audit_file(&seal, &integrity_paths.seal_path)?;

    let integrity_key = read_audit_integrity_key(&integrity_paths.key_path)?;
    let verification_result =
        verify_audit_integrity_state(journal_path, &integrity_paths, &integrity_key);
    let report_result = verification_result.map(|verification| {
        report_from_audit_integrity_verification(
            journal_path,
            integrity_paths.clone(),
            verification,
        )
    });
    let unlock_seal_result = unlock_audit_file(&seal, &integrity_paths.seal_path);
    let unlock_integrity_result =
        unlock_audit_file(&integrity_journal, &integrity_paths.integrity_journal_path);

    match (report_result, unlock_seal_result, unlock_integrity_result) {
        (Err(error), _, _) => Err(error),
        (Ok(_report), Err(error), _) => Err(error),
        (Ok(_report), Ok(()), Err(error)) => Err(error),
        (Ok(report), Ok(()), Ok(())) => Ok(report),
    }
}

fn report_from_audit_integrity_verification(
    journal_path: &Path,
    integrity_paths: AuditIntegrityPaths,
    verification: AuditIntegrityVerificationOutcome,
) -> AuditJournalIntegrityReport {
    match verification {
        AuditIntegrityVerificationOutcome::Verified(state) => AuditJournalIntegrityReport {
            journal_path: journal_path.to_path_buf(),
            paths: integrity_paths,
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            status: AuditJournalIntegrityStatus::Verified,
        },
        AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries,
            last_event_id,
            line,
            reason,
        } => AuditJournalIntegrityReport {
            journal_path: journal_path.to_path_buf(),
            paths: integrity_paths,
            protected_entries,
            last_event_id,
            status: AuditJournalIntegrityStatus::Mismatch { line, reason },
        },
    }
}

fn validate_existing_audit_journal(journal_path: &Path) -> Result<(), AuditError> {
    if !journal_path.exists() {
        return Err(AuditError::Sink(format!(
            "audit journal {} does not exist",
            journal_path.display()
        )));
    }

    if !journal_path.is_file() {
        return Err(AuditError::Sink(format!(
            "audit journal {} exists but is not a regular file",
            journal_path.display()
        )));
    }

    Ok(())
}

fn open_existing_read_write_file(path: &Path, label: &str) -> Result<File, AuditError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            AuditError::Sink(format!(
                "failed to open {label} `{}`: {error}",
                path.display()
            ))
        })
}

fn collect_missing_integrity_paths(paths: &AuditIntegrityPaths) -> Vec<String> {
    let candidates = [
        &paths.integrity_journal_path,
        &paths.key_path,
        &paths.seal_path,
    ];

    let mut missing_paths = Vec::new();

    for path in candidates {
        if !path.exists() {
            missing_paths.push(path.display().to_string());
        }
    }

    missing_paths
}

fn initialize_audit_integrity_state(
    journal_path: &Path,
    paths: &AuditIntegrityPaths,
    integrity_key: &[u8],
) -> Result<(), AuditError> {
    let integrity_journal_missing = !paths.integrity_journal_path.exists();
    let seal_missing = !paths.seal_path.exists();
    let key_missing = !paths.key_path.exists();
    let missing_paths = collect_missing_integrity_paths(paths);

    if missing_paths.is_empty() {
        let _ = load_audit_integrity_state(journal_path, paths, integrity_key)?;
        return Ok(());
    }

    if integrity_journal_missing && seal_missing && !key_missing {
        return rebuild_audit_integrity_state(journal_path, paths, integrity_key);
    }

    Err(AuditError::Sink(format!(
        "audit integrity sidecar for `{}` is partially missing: {}",
        journal_path.display(),
        missing_paths.join(", ")
    )))
}

fn rebuild_audit_integrity_state(
    journal_path: &Path,
    paths: &AuditIntegrityPaths,
    integrity_key: &[u8],
) -> Result<(), AuditError> {
    let event_lines = read_raw_jsonl_lines(journal_path)?;
    let mut state = AuditIntegrityState::default();
    let mut integrity_bytes = Vec::new();

    for line_bytes in &event_lines {
        let event = decode_audit_event_from_line(line_bytes, journal_path, state.entry_count + 1)?;
        let line_sha256 = compute_sha256(line_bytes);
        let chain_hmac = compute_chain_hmac(integrity_key, &state.last_chain_hmac, &line_sha256)?;
        let record = AuditIntegrityRecord {
            event_id: event.event_id.clone(),
            line_sha256_hex: hex_string(&line_sha256),
            chain_hmac_hex: hex_string(&chain_hmac),
        };
        let encoded_record =
            serialize_audit_integrity_record_line(&record, &paths.integrity_journal_path)?;
        integrity_bytes.extend_from_slice(&encoded_record);

        let line_len = line_bytes.len() as u64;
        let record_len = encoded_record.len() as u64;

        state.entry_count += 1;
        state.journal_bytes += line_len;
        state.integrity_bytes += record_len;
        state.last_event_id = Some(event.event_id);
        state.last_chain_hmac = chain_hmac;
    }

    let seal = build_audit_integrity_seal(&state, integrity_key)?;
    let seal_bytes = serialize_audit_integrity_seal(&seal, &paths.seal_path)?;
    state.seal_bytes = seal_bytes.len() as u64;

    fs::write(&paths.integrity_journal_path, &integrity_bytes).map_err(|error| {
        AuditError::Sink(format!(
            "failed to write audit integrity journal `{}`: {error}",
            paths.integrity_journal_path.display()
        ))
    })?;
    fs::write(&paths.seal_path, &seal_bytes).map_err(|error| {
        AuditError::Sink(format!(
            "failed to write audit integrity seal `{}`: {error}",
            paths.seal_path.display()
        ))
    })?;

    Ok(())
}

fn build_audit_integrity_seal(
    state: &AuditIntegrityState,
    integrity_key: &[u8],
) -> Result<AuditIntegritySeal, AuditError> {
    let last_chain_hmac_hex = hex_string(&state.last_chain_hmac);
    let unsigned = AuditIntegritySealUnsigned {
        entry_count: state.entry_count,
        journal_bytes: state.journal_bytes,
        integrity_bytes: state.integrity_bytes,
        last_event_id: state.last_event_id.as_deref(),
        last_chain_hmac_hex: &last_chain_hmac_hex,
    };
    let unsigned_bytes = serde_json::to_vec(&unsigned).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit integrity seal payload: {error}"
        ))
    })?;
    let seal_hmac = compute_hmac(integrity_key, &unsigned_bytes)?;

    Ok(AuditIntegritySeal {
        entry_count: state.entry_count,
        journal_bytes: state.journal_bytes,
        integrity_bytes: state.integrity_bytes,
        last_event_id: state.last_event_id.clone(),
        last_chain_hmac_hex,
        seal_hmac_hex: hex_string(&seal_hmac),
    })
}

fn load_audit_integrity_state(
    journal_path: &Path,
    paths: &AuditIntegrityPaths,
    integrity_key: &[u8],
) -> Result<AuditIntegrityState, AuditError> {
    let verification = verify_audit_integrity_state(journal_path, paths, integrity_key)?;

    match verification {
        AuditIntegrityVerificationOutcome::Verified(state) => Ok(state),
        AuditIntegrityVerificationOutcome::Mismatch { line, reason, .. } => {
            let line = line
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned());
            Err(AuditError::Sink(format!(
                "audit integrity mismatch for `{}` at line {}: {}",
                journal_path.display(),
                line,
                reason
            )))
        }
    }
}

fn verify_audit_integrity_state(
    journal_path: &Path,
    paths: &AuditIntegrityPaths,
    integrity_key: &[u8],
) -> Result<AuditIntegrityVerificationOutcome, AuditError> {
    let event_lines = read_raw_jsonl_lines(journal_path)?;
    let integrity_lines = read_raw_jsonl_lines(&paths.integrity_journal_path)?;
    let event_line_count = event_lines.len();
    let integrity_line_count = integrity_lines.len();

    let seal_bytes = fs::read(&paths.seal_path).map_err(|error| {
        AuditError::Sink(format!(
            "failed to read audit integrity seal `{}`: {error}",
            paths.seal_path.display()
        ))
    })?;
    let seal = serde_json::from_slice::<AuditIntegritySeal>(&seal_bytes).map_err(|error| {
        AuditError::Sink(format!(
            "failed to decode audit integrity seal `{}`: {error}",
            paths.seal_path.display()
        ))
    })?;
    verify_audit_integrity_seal(&seal, integrity_key, &paths.seal_path)?;

    let mut state = AuditIntegrityState::default();
    let mut journal_bytes = 0_u64;
    let mut integrity_bytes = 0_u64;

    for (index, (event_line, integrity_line)) in
        event_lines.iter().zip(integrity_lines.iter()).enumerate()
    {
        let line_number = index + 1;
        let event = decode_audit_event_from_line(event_line, journal_path, line_number)?;
        let record = decode_audit_integrity_record(integrity_line, paths, line_number)?;

        if record.event_id != event.event_id {
            let reason = format!(
                "event_id journal={} integrity={}",
                event.event_id, record.event_id
            );
            let outcome = AuditIntegrityVerificationOutcome::Mismatch {
                protected_entries: state.entry_count,
                last_event_id: state.last_event_id,
                line: Some(line_number),
                reason,
            };
            return Ok(outcome);
        }

        let line_sha256 = compute_sha256(event_line);
        let line_sha256_hex = hex_string(&line_sha256);

        if record.line_sha256_hex != line_sha256_hex {
            let reason = "line hash does not match".to_owned();
            let outcome = AuditIntegrityVerificationOutcome::Mismatch {
                protected_entries: state.entry_count,
                last_event_id: state.last_event_id,
                line: Some(line_number),
                reason,
            };
            return Ok(outcome);
        }

        let expected_chain_hmac =
            compute_chain_hmac(integrity_key, &state.last_chain_hmac, &line_sha256)?;
        let expected_chain_hmac_hex = hex_string(&expected_chain_hmac);

        if record.chain_hmac_hex != expected_chain_hmac_hex {
            let reason = "chain HMAC does not match".to_owned();
            let outcome = AuditIntegrityVerificationOutcome::Mismatch {
                protected_entries: state.entry_count,
                last_event_id: state.last_event_id,
                line: Some(line_number),
                reason,
            };
            return Ok(outcome);
        }

        state.entry_count += 1;
        state.last_event_id = Some(event.event_id);
        state.last_chain_hmac = expected_chain_hmac;
        journal_bytes += event_line.len() as u64;
        integrity_bytes += integrity_line.len() as u64;
    }

    if event_line_count != integrity_line_count {
        let reason = format!(
            "journal lines={} integrity lines={}",
            event_line_count, integrity_line_count
        );
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: Some(state.entry_count + 1),
            reason,
        };
        return Ok(outcome);
    }

    let expected_last_chain_hmac_hex = hex_string(&state.last_chain_hmac);

    if seal.entry_count != state.entry_count {
        let reason = format!(
            "seal entry_count expected={} actual={}",
            seal.entry_count, state.entry_count
        );
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: None,
            reason,
        };
        return Ok(outcome);
    }

    if seal.journal_bytes != journal_bytes {
        let reason = format!(
            "seal journal_bytes expected={} actual={}",
            seal.journal_bytes, journal_bytes
        );
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: None,
            reason,
        };
        return Ok(outcome);
    }

    if seal.integrity_bytes != integrity_bytes {
        let reason = format!(
            "seal integrity_bytes expected={} actual={}",
            seal.integrity_bytes, integrity_bytes
        );
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: None,
            reason,
        };
        return Ok(outcome);
    }

    if seal.last_event_id != state.last_event_id {
        let reason = "seal last_event_id does not match".to_owned();
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: None,
            reason,
        };
        return Ok(outcome);
    }

    if seal.last_chain_hmac_hex != expected_last_chain_hmac_hex {
        let reason = "seal last_chain_hmac does not match".to_owned();
        let outcome = AuditIntegrityVerificationOutcome::Mismatch {
            protected_entries: state.entry_count,
            last_event_id: state.last_event_id,
            line: None,
            reason,
        };
        return Ok(outcome);
    }

    state.journal_bytes = journal_bytes;
    state.integrity_bytes = integrity_bytes;
    state.seal_bytes = seal_bytes.len() as u64;

    Ok(AuditIntegrityVerificationOutcome::Verified(state))
}

fn verify_audit_integrity_seal(
    seal: &AuditIntegritySeal,
    integrity_key: &[u8],
    seal_path: &Path,
) -> Result<(), AuditError> {
    let unsigned = AuditIntegritySealUnsigned {
        entry_count: seal.entry_count,
        journal_bytes: seal.journal_bytes,
        integrity_bytes: seal.integrity_bytes,
        last_event_id: seal.last_event_id.as_deref(),
        last_chain_hmac_hex: &seal.last_chain_hmac_hex,
    };
    let unsigned_bytes = serde_json::to_vec(&unsigned).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit integrity seal payload for `{}`: {error}",
            seal_path.display()
        ))
    })?;
    let expected_hmac = compute_hmac(integrity_key, &unsigned_bytes)?;
    let expected_hmac_hex = hex_string(&expected_hmac);

    if seal.seal_hmac_hex != expected_hmac_hex {
        return Err(AuditError::Sink(format!(
            "audit integrity seal mismatch for `{}`: seal HMAC does not match",
            seal_path.display()
        )));
    }

    Ok(())
}

fn read_raw_jsonl_lines(path: &Path) -> Result<Vec<Vec<u8>>, AuditError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path).map_err(|error| {
        AuditError::Sink(format!("failed to open `{}`: {error}", path.display()))
    })?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();

    loop {
        let mut line_bytes = Vec::new();
        let read = reader.read_until(b'\n', &mut line_bytes).map_err(|error| {
            AuditError::Sink(format!("failed to read `{}`: {error}", path.display()))
        })?;

        if read == 0 {
            break;
        }

        lines.push(line_bytes);
    }

    Ok(lines)
}

fn decode_audit_event_from_line(
    line_bytes: &[u8],
    path: &Path,
    line_number: usize,
) -> Result<AuditEvent, AuditError> {
    let trimmed = trim_jsonl_line_bytes(line_bytes);

    serde_json::from_slice::<AuditEvent>(trimmed).map_err(|error| {
        AuditError::Sink(format!(
            "failed to decode audit event from `{}` line {}: {error}",
            path.display(),
            line_number
        ))
    })
}

fn decode_audit_integrity_record(
    line_bytes: &[u8],
    paths: &AuditIntegrityPaths,
    line_number: usize,
) -> Result<AuditIntegrityRecord, AuditError> {
    let trimmed = trim_jsonl_line_bytes(line_bytes);

    serde_json::from_slice::<AuditIntegrityRecord>(trimmed).map_err(|error| {
        AuditError::Sink(format!(
            "failed to decode audit integrity record from `{}` line {}: {error}",
            paths.integrity_journal_path.display(),
            line_number
        ))
    })
}

fn trim_jsonl_line_bytes(line_bytes: &[u8]) -> &[u8] {
    line_bytes.strip_suffix(b"\n").unwrap_or(line_bytes)
}

fn compute_sha256(message: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(message);
    digest.into()
}

fn compute_chain_hmac(
    integrity_key: &[u8],
    previous_chain_hmac: &[u8; 32],
    line_sha256: &[u8; 32],
) -> Result<[u8; 32], AuditError> {
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(previous_chain_hmac);
    message.extend_from_slice(line_sha256);
    compute_hmac(integrity_key, &message)
}

fn compute_hmac(integrity_key: &[u8], message: &[u8]) -> Result<[u8; 32], AuditError> {
    let mac_result = HmacSha256::new_from_slice(integrity_key);
    let mut mac = mac_result.map_err(|error| {
        AuditError::Sink(format!(
            "failed to initialize audit integrity HMAC: {error}"
        ))
    })?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().into())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        use std::fmt::Write as _;

        let _ = write!(&mut output, "{byte:02x}");
    }

    output
}

impl AuditSink for JsonlAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_error| AuditError::Sink("audit journal mutex poisoned".to_owned()))?;
        let mut integrity_journal = self.integrity_journal.lock().map_err(|_error| {
            AuditError::Sink("audit integrity journal mutex poisoned".to_owned())
        })?;
        let mut integrity_seal = self
            .integrity_seal
            .lock()
            .map_err(|_error| AuditError::Sink("audit integrity seal mutex poisoned".to_owned()))?;
        let mut integrity_state = self.integrity_state.lock().map_err(|_error| {
            AuditError::Sink("audit integrity state mutex poisoned".to_owned())
        })?;

        let encoded = serialize_audit_event_line(&event, &self.path)?;
        let line_sha256 = compute_sha256(&encoded);
        let chain_hmac = compute_chain_hmac(
            &self.integrity_key,
            &integrity_state.last_chain_hmac,
            &line_sha256,
        )?;
        let integrity_record = AuditIntegrityRecord {
            event_id: event.event_id.clone(),
            line_sha256_hex: hex_string(&line_sha256),
            chain_hmac_hex: hex_string(&chain_hmac),
        };
        let encoded_integrity = serialize_audit_integrity_record_line(
            &integrity_record,
            &self.integrity_paths.integrity_journal_path,
        )?;

        let previous_state = integrity_state.clone();

        lock_audit_file(&journal, &self.path)?;
        lock_audit_file(
            &integrity_journal,
            &self.integrity_paths.integrity_journal_path,
        )?;
        lock_audit_file(&integrity_seal, &self.integrity_paths.seal_path)?;

        let write_result = journal
            .write_all(&encoded)
            .map_err(|error| {
                AuditError::Sink(format!(
                    "failed to append audit event to `{}`: {error}",
                    self.path.display()
                ))
            })
            .and_then(|()| {
                journal.flush().map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to flush audit journal `{}`: {error}",
                        self.path.display()
                    ))
                })
            })
            .and_then(|()| {
                integrity_journal
                    .write_all(&encoded_integrity)
                    .map_err(|error| {
                        AuditError::Sink(format!(
                            "failed to append audit integrity record to `{}`: {error}",
                            self.integrity_paths.integrity_journal_path.display()
                        ))
                    })
            })
            .and_then(|()| {
                integrity_journal.flush().map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to flush audit integrity journal `{}`: {error}",
                        self.integrity_paths.integrity_journal_path.display()
                    ))
                })
            })
            .and_then(|()| {
                let mut next_state = previous_state.clone();
                next_state.entry_count += 1;
                next_state.journal_bytes += encoded.len() as u64;
                next_state.integrity_bytes += encoded_integrity.len() as u64;
                next_state.last_event_id = Some(event.event_id.clone());
                next_state.last_chain_hmac = chain_hmac;

                let seal = build_audit_integrity_seal(&next_state, &self.integrity_key)?;
                let encoded_seal =
                    serialize_audit_integrity_seal(&seal, &self.integrity_paths.seal_path)?;

                integrity_seal.set_len(0).map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to truncate audit integrity seal `{}`: {error}",
                        self.integrity_paths.seal_path.display()
                    ))
                })?;
                integrity_seal.seek(SeekFrom::Start(0)).map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to rewind audit integrity seal `{}`: {error}",
                        self.integrity_paths.seal_path.display()
                    ))
                })?;
                integrity_seal.write_all(&encoded_seal).map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to write audit integrity seal `{}`: {error}",
                        self.integrity_paths.seal_path.display()
                    ))
                })?;
                integrity_seal.flush().map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to flush audit integrity seal `{}`: {error}",
                        self.integrity_paths.seal_path.display()
                    ))
                })?;

                next_state.seal_bytes = encoded_seal.len() as u64;
                *integrity_state = next_state;

                Ok(())
            });

        if write_result.is_err() {
            let _ = journal.set_len(previous_state.journal_bytes);
            let _ = journal.seek(SeekFrom::Start(previous_state.journal_bytes));
            let _ = integrity_journal.set_len(previous_state.integrity_bytes);
            let _ = integrity_journal.seek(SeekFrom::Start(previous_state.integrity_bytes));
            let _ = integrity_seal.set_len(previous_state.seal_bytes);
            let _ = integrity_seal.seek(SeekFrom::Start(previous_state.seal_bytes));
            *integrity_state = previous_state;
        }

        let unlock_seal_result =
            unlock_audit_file(&integrity_seal, &self.integrity_paths.seal_path);
        let unlock_integrity_result = unlock_audit_file(
            &integrity_journal,
            &self.integrity_paths.integrity_journal_path,
        );
        let unlock_journal_result = unlock_audit_file(&journal, &self.path);

        match (
            write_result,
            unlock_seal_result,
            unlock_integrity_result,
            unlock_journal_result,
        ) {
            (Err(error), _, _, _) => Err(error),
            (Ok(()), Err(error), _, _) => Err(error),
            (Ok(()), Ok(()), Err(error), _) => Err(error),
            (Ok(()), Ok(()), Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(()), Ok(()), Ok(())) => Ok(()),
        }
    }
}

pub struct FanoutAuditSink {
    children: Vec<Arc<dyn AuditSink>>,
}

impl FanoutAuditSink {
    #[must_use]
    pub fn new(children: Vec<Arc<dyn AuditSink>>) -> Self {
        assert!(
            !children.is_empty(),
            "fanout audit sink requires at least one child"
        );
        Self { children }
    }
}

impl AuditSink for FanoutAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        if let Some((last, rest)) = self.children.split_last() {
            for sink in rest {
                sink.record(event.clone())?;
            }
            last.record(event)?;
        }
        Ok(())
    }
}
