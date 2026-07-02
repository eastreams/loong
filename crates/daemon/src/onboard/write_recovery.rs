use std::fs;
use std::path::{Path, PathBuf};

use loong_spec::CliResult;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]-[hour][minute][second]");

#[derive(Debug, Clone)]
pub(crate) struct ConfigWritePlan {
    pub(crate) force: bool,
    pub(crate) backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct OnboardWriteRecovery {
    pub(crate) output_preexisted: bool,
    pub(crate) backup_path: Option<PathBuf>,
    pub(crate) keep_backup_on_success: bool,
}

pub(crate) fn prepare_output_path_for_write(
    output_path: &Path,
    plan: &ConfigWritePlan,
) -> CliResult<OnboardWriteRecovery> {
    let output_preexisted = output_path.exists();
    let keep_backup_on_success = plan.backup_path.is_some();
    let backup_path = if output_preexisted {
        let resolved_backup_path = plan
            .backup_path
            .clone()
            .unwrap_or(resolve_rollback_backup_path(output_path)?);
        Some(resolved_backup_path)
    } else {
        None
    };

    if let Some(backup_path) = backup_path.as_deref() {
        backup_existing_config(output_path, backup_path)?;
    }

    Ok(OnboardWriteRecovery {
        output_preexisted,
        backup_path,
        keep_backup_on_success,
    })
}

pub fn backup_existing_config(output_path: &Path, backup_path: &Path) -> CliResult<()> {
    fs::copy(output_path, backup_path)
        .map_err(|error| format!("failed to backup config: {error}"))?;
    Ok(())
}

impl OnboardWriteRecovery {
    pub(crate) fn rollback(&self, output_path: &Path) -> CliResult<()> {
        if self.output_preexisted {
            let backup_path = self
                .backup_path
                .as_deref()
                .ok_or_else(|| "missing rollback backup for existing config".to_owned())?;

            fs::copy(backup_path, output_path).map_err(|error| {
                format!(
                    "failed to restore original config {} from backup {}: {error}",
                    output_path.display(),
                    backup_path.display(),
                )
            })?;
            self.finish_success();
            return Ok(());
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|error| {
                format!(
                    "failed to remove partial config {} after onboarding failure: {error}",
                    output_path.display()
                )
            })?;
        }

        self.finish_success();
        Ok(())
    }

    pub(crate) fn finish_success(&self) {
        if self.keep_backup_on_success {
            return;
        }

        if let Some(backup_path) = self.backup_path.as_deref() {
            let _ = fs::remove_file(backup_path);
        }
    }
}

pub(crate) fn rollback_onboard_write_failure(
    output_path: &Path,
    write_recovery: &OnboardWriteRecovery,
    failure: impl Into<String>,
) -> String {
    let failure = failure.into();
    let rollback_result = write_recovery.rollback(output_path);

    match rollback_result {
        Ok(()) => failure,
        Err(rollback_error) => {
            format!("{failure}; additionally failed to restore original config: {rollback_error}")
        }
    }
}

pub(crate) fn resolve_backup_path(original: &Path) -> CliResult<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_backup_path_at(original, now)
}

pub(crate) fn resolve_backup_path_at(
    original: &Path,
    timestamp: OffsetDateTime,
) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_stem = original
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_owned());
    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;

    Ok(parent.join(format!("{}.toml.bak-{}", file_stem, formatted_timestamp)))
}

pub(crate) fn resolve_rollback_backup_path(original: &Path) -> CliResult<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_rollback_backup_path_at(original, now)
}

pub(crate) fn resolve_rollback_backup_path_at(
    original: &Path,
    timestamp: OffsetDateTime,
) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_name = original
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_owned());
    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;

    Ok(parent.join(format!(
        ".{file_name}.onboard-rollback-{formatted_timestamp}"
    )))
}

pub(crate) fn format_backup_timestamp_at(timestamp: OffsetDateTime) -> CliResult<String> {
    timestamp
        .format(BACKUP_TIMESTAMP_FORMAT)
        .map_err(|error| format!("format backup timestamp failed: {error}"))
}
