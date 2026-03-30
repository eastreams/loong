// Functions here are temporarily unused after removing GuidedOnboardUiRunner.
#![allow(dead_code)]
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
    pub persist_displayed_file_root: bool,
}

pub(super) fn derive_workspace_step_values(
    draft: &OnboardDraft,
    context: &OnboardRuntimeContext,
) -> WorkspaceStepValues {
    let sqlite_path = draft.workspace.sqlite_path.clone();
    let sqlite_origin = draft.origin_for(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY);

    let explicit_file_root = draft.config.tools.explicit_file_root();
    let seeded_file_root_origin = draft.origin_for(OnboardDraft::WORKSPACE_FILE_ROOT_KEY);
    let uses_workspace_root_default =
        explicit_file_root.is_none() && context.workspace_root.is_some();
    let persist_displayed_file_root = explicit_file_root.is_some()
        || (uses_workspace_root_default
            && seeded_file_root_origin != Some(OnboardValueOrigin::CurrentSetup));
    let (file_root, file_root_origin) = if explicit_file_root.is_none() {
        if let Some(workspace_root) = context.workspace_root.clone() {
            (
                workspace_root,
                seeded_file_root_origin.or(Some(OnboardValueOrigin::DetectedStartingPoint)),
            )
        } else {
            (draft.workspace.file_root.clone(), seeded_file_root_origin)
        }
    } else {
        (draft.workspace.file_root.clone(), seeded_file_root_origin)
    };

    WorkspaceStepValues {
        sqlite_path,
        sqlite_origin,
        file_root,
        file_root_origin,
        persist_displayed_file_root,
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
    if values.persist_displayed_file_root {
        draft.config.tools.file_root = Some(values.file_root.display().to_string());
        if let Some(origin) = values.file_root_origin {
            draft
                .origins
                .insert(OnboardDraft::WORKSPACE_FILE_ROOT_KEY, origin);
        }
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
        persist_displayed_file_root: displayed_values.persist_displayed_file_root,
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
        Ok(_) if !sqlite_file_is_writable(sqlite_path) => {
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

fn sqlite_file_is_writable(sqlite_path: &Path) -> bool {
    fs::OpenOptions::new().write(true).open(sqlite_path).is_ok()
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

    // This is a best-effort preflight heuristic based on Unix mode bits, not the
    // effective permissions of the current process. Real filesystem operations
    // still surface the authoritative EACCES-style failures when they happen.
    permissions.mode() & 0o222 == 0
}

#[cfg(not(unix))]
fn directory_permissions_block_write(permissions: &fs::Permissions) -> bool {
    permissions.readonly()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loongclaw_app as mvp;

    fn test_context(workspace_root: Option<PathBuf>) -> OnboardRuntimeContext {
        OnboardRuntimeContext::new_for_tests(80, workspace_root, std::iter::empty::<PathBuf>())
    }

    #[test]
    fn blank_current_file_root_stays_implicit_when_workspace_root_is_only_display_default() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(String::new());
        let mut draft = OnboardDraft::from_config(
            config,
            PathBuf::from("loongclaw.toml"),
            Some(OnboardValueOrigin::CurrentSetup),
        );
        let workspace_root = std::env::temp_dir().join("loongclaw-current-workspace-root");

        let displayed_values =
            derive_workspace_step_values(&draft, &test_context(Some(workspace_root.clone())));

        assert_eq!(displayed_values.file_root, workspace_root);
        assert_eq!(
            displayed_values.file_root_origin,
            Some(OnboardValueOrigin::CurrentSetup)
        );
        assert!(
            !displayed_values.persist_displayed_file_root,
            "reruns with an implicit current file root should not materialize it as an explicit value"
        );

        apply_workspace_step_values(&mut draft, &displayed_values);
        assert_eq!(draft.config.tools.file_root.as_deref(), Some(""));
    }

    #[test]
    fn start_fresh_workspace_root_default_is_persistable() {
        let mut draft = OnboardDraft::from_config(
            mvp::config::LoongClawConfig::default(),
            PathBuf::from("loongclaw.toml"),
            None,
        );
        let workspace_root = std::env::temp_dir().join("loongclaw-start-fresh-workspace-root");

        let displayed_values =
            derive_workspace_step_values(&draft, &test_context(Some(workspace_root.clone())));

        assert_eq!(
            displayed_values.file_root_origin,
            Some(OnboardValueOrigin::DetectedStartingPoint)
        );
        assert!(
            displayed_values.persist_displayed_file_root,
            "fresh onboarding should persist the workspace-root-backed default when the user keeps it"
        );

        apply_workspace_step_values(&mut draft, &displayed_values);
        let workspace_root_display = workspace_root.display().to_string();
        assert_eq!(
            draft.config.tools.file_root.as_deref(),
            Some(workspace_root_display.as_str())
        );
    }
}
