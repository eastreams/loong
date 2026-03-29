use std::fs;
use std::path::{Path, PathBuf};

use super::OnboardRuntimeContext;
use crate::CliResult;
use crate::onboard_preflight::{OnboardCheckLevel, directory_preflight_check};
use crate::onboard_state::{OnboardDraft, OnboardValueOrigin};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceStepValues {
    pub sqlite_path: PathBuf,
    pub sqlite_origin: Option<OnboardValueOrigin>,
    pub file_root: PathBuf,
    pub file_root_origin: Option<OnboardValueOrigin>,
}

pub(super) fn derive_workspace_step_values(
    draft: &OnboardDraft,
    context: &OnboardRuntimeContext,
) -> WorkspaceStepValues {
    let sqlite_path = draft.workspace.sqlite_path.clone();
    let sqlite_origin = draft.origin_for(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY);

    let explicit_file_root = draft
        .config
        .tools
        .file_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (file_root, file_root_origin) =
        if explicit_file_root.is_none() && context.workspace_root.is_some() {
            (
                context
                    .workspace_root
                    .clone()
                    .unwrap_or_else(|| draft.workspace.file_root.clone()),
                Some(OnboardValueOrigin::DetectedStartingPoint),
            )
        } else {
            (
                draft.workspace.file_root.clone(),
                draft.origin_for(OnboardDraft::WORKSPACE_FILE_ROOT_KEY),
            )
        };

    WorkspaceStepValues {
        sqlite_path,
        sqlite_origin,
        file_root,
        file_root_origin,
    }
}

pub(super) fn apply_workspace_step_values(draft: &mut OnboardDraft, values: &WorkspaceStepValues) {
    draft.workspace.sqlite_path = values.sqlite_path.clone();
    draft.config.memory.sqlite_path = values.sqlite_path.display().to_string();
    if let Some(origin) = values.sqlite_origin {
        draft
            .origins
            .insert(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY, origin);
    }

    draft.workspace.file_root = values.file_root.clone();
    draft.config.tools.file_root = Some(values.file_root.display().to_string());
    if let Some(origin) = values.file_root_origin {
        draft
            .origins
            .insert(OnboardDraft::WORKSPACE_FILE_ROOT_KEY, origin);
    }
}

pub(super) fn selected_workspace_step_values(
    displayed_values: &WorkspaceStepValues,
    sqlite_path: PathBuf,
    file_root: PathBuf,
) -> WorkspaceStepValues {
    WorkspaceStepValues {
        sqlite_path,
        sqlite_origin: displayed_values.sqlite_origin,
        file_root,
        file_root_origin: displayed_values.file_root_origin,
    }
}

pub(super) fn commit_workspace_step_selection(
    draft: &mut OnboardDraft,
    displayed_values: &WorkspaceStepValues,
    selected_values: &WorkspaceStepValues,
) {
    apply_workspace_step_values(draft, displayed_values);

    if selected_values.sqlite_path != displayed_values.sqlite_path {
        draft.set_workspace_sqlite_path(selected_values.sqlite_path.clone());
    }
    if selected_values.file_root != displayed_values.file_root {
        draft.set_workspace_file_root(selected_values.file_root.clone());
    }
}

pub(super) fn validate_workspace_step_values(values: &WorkspaceStepValues) -> CliResult<()> {
    validate_sqlite_path(values.sqlite_path.as_path())?;
    validate_directory_target("tool file root", values.file_root.as_path())
}

fn validate_sqlite_path(sqlite_path: &Path) -> CliResult<()> {
    match fs::metadata(sqlite_path) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(format!(
                "workspace step blocked: sqlite memory path {} exists but is a directory",
                sqlite_path.display()
            ));
        }
        Ok(metadata) if metadata.permissions().readonly() => {
            return Err(format!(
                "workspace step blocked: sqlite memory path {} is not writable",
                sqlite_path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "workspace step blocked: failed to inspect sqlite memory path {}: {error}",
                sqlite_path.display()
            ));
        }
    }

    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    validate_directory_target("sqlite memory path", sqlite_parent)
}

fn validate_directory_target(name: &'static str, target: &Path) -> CliResult<()> {
    let check = directory_preflight_check(name, target);
    if check.level == OnboardCheckLevel::Fail {
        return Err(format!("workspace step blocked: {}", check.detail));
    }

    if let Some(issue) = directory_writability_issue(target) {
        return Err(format!("workspace step blocked: {issue}"));
    }

    Ok(())
}

fn directory_writability_issue(target: &Path) -> Option<String> {
    let probe_target = nearest_existing_ancestor(target)?;
    let metadata = fs::metadata(probe_target).ok()?;
    if !metadata.is_dir() {
        return Some(format!(
            "{} exists but is not a directory",
            probe_target.display()
        ));
    }

    if directory_permissions_block_write(&metadata.permissions()) {
        return Some(format!("{} is not writable", probe_target.display()));
    }

    None
}

fn nearest_existing_ancestor(target: &Path) -> Option<&Path> {
    let mut ancestor = target;
    while !ancestor.exists() {
        ancestor = ancestor.parent()?;
    }
    Some(ancestor)
}

#[cfg(unix)]
fn directory_permissions_block_write(permissions: &fs::Permissions) -> bool {
    use std::os::unix::fs::PermissionsExt;

    permissions.mode() & 0o222 == 0
}

#[cfg(not(unix))]
fn directory_permissions_block_write(permissions: &fs::Permissions) -> bool {
    permissions.readonly()
}
