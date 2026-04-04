use loongclaw_spec::CliResult;

use crate::kernel;
use crate::mvp;

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotAuditState {
    pub mode: mvp::config::AuditMode,
    pub journal_path: String,
    pub integrity_journal_path: String,
    pub integrity_key_path: String,
    pub integrity_seal_path: String,
    pub integrity_status: String,
    pub protected_entries: usize,
    pub last_event_id: Option<String>,
    pub integrity_detail: String,
}

pub fn collect_runtime_snapshot_audit_state(
    config: &mvp::config::LoongClawConfig,
) -> CliResult<RuntimeSnapshotAuditState> {
    let audit = &config.audit;
    let journal_path = audit.resolved_path();
    let integrity_paths = kernel::derive_jsonl_audit_integrity_paths(&journal_path);
    let journal_missing = !journal_path.exists();

    let verification = if matches!(audit.mode, mvp::config::AuditMode::InMemory) || journal_missing
    {
        None
    } else {
        let report = kernel::verify_jsonl_audit_journal_integrity(&journal_path)
            .map_err(|error| format!("collect audit integrity snapshot failed: {error}"))?;
        Some(report)
    };

    let integrity_status = if matches!(audit.mode, mvp::config::AuditMode::InMemory) {
        "disabled".to_owned()
    } else if journal_missing {
        "missing_artifacts".to_owned()
    } else {
        match &verification {
            None => "missing_artifacts".to_owned(),
            Some(report) => match &report.status {
                kernel::AuditJournalIntegrityStatus::Verified => "verified".to_owned(),
                kernel::AuditJournalIntegrityStatus::MissingArtifacts { .. } => {
                    "missing_artifacts".to_owned()
                }
                kernel::AuditJournalIntegrityStatus::Mismatch { .. } => "mismatch".to_owned(),
            },
        }
    };

    let protected_entries = verification
        .as_ref()
        .map(|report| report.protected_entries)
        .unwrap_or(0);
    let last_event_id = verification
        .as_ref()
        .and_then(|report| report.last_event_id.clone());
    let integrity_detail = if matches!(audit.mode, mvp::config::AuditMode::InMemory) {
        "audit integrity disabled because [audit].mode = \"in_memory\"".to_owned()
    } else if journal_missing {
        format!(
            "audit journal {} has not been created yet; integrity sidecar will appear after the first durable audit write",
            journal_path.display()
        )
    } else {
        match &verification {
            Some(report) => match &report.status {
                kernel::AuditJournalIntegrityStatus::Verified => format!(
                    "verified tamper-evident audit sidecar protected_entries={} last_event_id={}",
                    report.protected_entries,
                    report.last_event_id.as_deref().unwrap_or("-")
                ),
                kernel::AuditJournalIntegrityStatus::MissingArtifacts { missing_paths } => format!(
                    "missing integrity sidecar artifacts: {}",
                    missing_paths.join(", ")
                ),
                kernel::AuditJournalIntegrityStatus::Mismatch { line, reason } => {
                    let line = line
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned());
                    format!("audit integrity mismatch at line {}: {}", line, reason)
                }
            },
            None => "missing integrity sidecar artifacts".to_owned(),
        }
    };

    Ok(RuntimeSnapshotAuditState {
        mode: audit.mode,
        journal_path: journal_path.display().to_string(),
        integrity_journal_path: integrity_paths.integrity_journal_path.display().to_string(),
        integrity_key_path: integrity_paths.key_path.display().to_string(),
        integrity_seal_path: integrity_paths.seal_path.display().to_string(),
        integrity_status,
        protected_entries,
        last_event_id,
        integrity_detail,
    })
}
