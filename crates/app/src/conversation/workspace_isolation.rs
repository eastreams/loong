use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::config::LoongClawConfig;

#[cfg(test)]
use super::ConstrainedSubagentMode;
use super::{ConstrainedSubagentExecution, ConstrainedSubagentIsolation};
#[cfg(test)]
use crate::tools::runtime_config::ToolRuntimeNarrowing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DelegateWorkspaceCleanupResult {
    pub workspace_root: PathBuf,
    pub retained: bool,
    pub dirty: bool,
}

pub(super) fn prepare_delegate_workspace_root(
    config: &LoongClawConfig,
    child_session_id: &str,
    isolation: ConstrainedSubagentIsolation,
) -> Result<Option<PathBuf>, String> {
    if isolation != ConstrainedSubagentIsolation::Worktree {
        return Ok(None);
    }

    let base_root = config.tools.resolved_file_root();
    let repo_root = resolve_git_repo_root(base_root.as_path())?;
    let worktrees_root = repo_root.join(".worktrees");
    std::fs::create_dir_all(&worktrees_root).map_err(|error| {
        let display_path = worktrees_root.display();
        format!("create delegate worktrees root `{display_path}` failed: {error}")
    })?;

    let worktree_name = sanitized_delegate_worktree_name(child_session_id);
    let worktree_root = worktrees_root.join(worktree_name);
    remove_stale_delegate_worktree(worktree_root.as_path())?;
    add_detached_worktree(repo_root.as_path(), worktree_root.as_path())?;

    let canonical_root = std::fs::canonicalize(worktree_root.as_path()).map_err(|error| {
        let display_path = worktree_root.display();
        format!("canonicalize delegate worktree `{display_path}` failed: {error}")
    })?;
    Ok(Some(canonical_root))
}

pub(super) fn cleanup_prepared_delegate_workspace_root(
    isolation: ConstrainedSubagentIsolation,
    workspace_root: Option<&Path>,
) -> Result<(), String> {
    if isolation != ConstrainedSubagentIsolation::Worktree {
        return Ok(());
    }

    let Some(workspace_root) = workspace_root else {
        return Ok(());
    };
    if !workspace_root.exists() {
        return Ok(());
    }

    remove_delegate_worktree(workspace_root)
}

pub(super) fn cleanup_delegate_workspace_root(
    execution: &ConstrainedSubagentExecution,
) -> Result<Option<DelegateWorkspaceCleanupResult>, String> {
    if execution.isolation != ConstrainedSubagentIsolation::Worktree {
        return Ok(None);
    }

    let Some(workspace_root) = execution.workspace_root.as_ref() else {
        return Ok(None);
    };
    let dirty = delegate_worktree_is_dirty(workspace_root.as_path())?;
    if dirty {
        return Ok(Some(DelegateWorkspaceCleanupResult {
            workspace_root: workspace_root.clone(),
            retained: true,
            dirty: true,
        }));
    }

    remove_delegate_worktree(workspace_root.as_path())?;
    Ok(Some(DelegateWorkspaceCleanupResult {
        workspace_root: workspace_root.clone(),
        retained: false,
        dirty: false,
    }))
}

fn resolve_git_repo_root(base_root: &Path) -> Result<PathBuf, String> {
    let args = [
        OsStr::new("-C"),
        base_root.as_os_str(),
        OsStr::new("rev-parse"),
        OsStr::new("--show-toplevel"),
    ];
    let output = run_git_command(&args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` failed: {stderr}"
        ));
    }

    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed_stdout = raw_stdout.trim();
    if trimmed_stdout.is_empty() {
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` returned empty output"
        ));
    }

    Ok(PathBuf::from(trimmed_stdout))
}

fn sanitized_delegate_worktree_name(child_session_id: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_was_dash = false;

    for ch in child_session_id.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
            previous_was_dash = false;
            continue;
        }

        if !previous_was_dash {
            sanitized.push('-');
            previous_was_dash = true;
        }
    }

    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        return "delegate-child".to_owned();
    }

    trimmed.to_owned()
}

fn remove_stale_delegate_worktree(worktree_root: &Path) -> Result<(), String> {
    if !worktree_root.exists() {
        return Ok(());
    }

    remove_delegate_worktree(worktree_root)
}

fn add_detached_worktree(repo_root: &Path, worktree_root: &Path) -> Result<(), String> {
    let args = [
        OsStr::new("-C"),
        repo_root.as_os_str(),
        OsStr::new("worktree"),
        OsStr::new("add"),
        OsStr::new("--detach"),
        worktree_root.as_os_str(),
        OsStr::new("HEAD"),
    ];
    let output = run_git_command(&args)?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let display_path = worktree_root.display();
    Err(format!(
        "create delegate worktree `{display_path}` failed: {stderr}"
    ))
}

fn remove_delegate_worktree(worktree_root: &Path) -> Result<(), String> {
    let args = [
        OsStr::new("-C"),
        worktree_root.as_os_str(),
        OsStr::new("worktree"),
        OsStr::new("remove"),
        OsStr::new("--force"),
        worktree_root.as_os_str(),
    ];
    let output = run_git_command(&args)?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let display_path = worktree_root.display();
    Err(format!(
        "remove delegate worktree `{display_path}` failed: {stderr}"
    ))
}

fn delegate_worktree_is_dirty(workspace_root: &Path) -> Result<bool, String> {
    let args = [
        OsStr::new("-C"),
        workspace_root.as_os_str(),
        OsStr::new("status"),
        OsStr::new("--porcelain"),
    ];
    let output = run_git_command(&args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let display_path = workspace_root.display();
        return Err(format!(
            "inspect delegate worktree status for `{display_path}` failed: {stderr}"
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(!stdout.trim().is_empty())
}

fn run_git_command(args: &[&OsStr]) -> Result<std::process::Output, String> {
    let git_executable = resolve_git_executable()?;
    Command::new(git_executable)
        .args(args)
        .output()
        .map_err(|error| format!("spawn git command failed: {error}"))
}

pub(crate) fn resolve_git_executable() -> Result<&'static Path, String> {
    static GIT_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = GIT_EXECUTABLE.get() {
        return Ok(path.as_path());
    }

    let discovered = discover_git_executable()?;
    let cached = GIT_EXECUTABLE.get_or_init(|| discovered);
    Ok(cached.as_path())
}

fn discover_git_executable() -> Result<PathBuf, String> {
    if let Ok(path) = which::which("git") {
        return Ok(path);
    }

    let stable_search_path = stable_command_search_path();
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let discovered = which::which_in("git", Some(stable_search_path), current_dir)
        .map_err(|error| format!("resolve git executable failed: {error}"))?;
    Ok(discovered)
}

fn stable_command_search_path() -> OsString {
    let env_path = std::env::var_os("PATH");
    let fallback = env_path
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin"));
    let output = Command::new("/usr/bin/getconf").arg("PATH").output();
    let Ok(output) = output else {
        return fallback;
    };
    if !output.status.success() {
        return fallback;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return fallback;
    }

    OsString::from(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(root: &Path) {
        std::fs::create_dir_all(root).expect("create git repo root");
        let root_string = root.display().to_string();
        let git_executable = resolve_git_executable().expect("resolve git executable");

        let init_status = Command::new(git_executable)
            .args(["-C", root_string.as_str(), "init", "-q"])
            .status()
            .expect("git init status");
        assert!(init_status.success(), "git init should succeed");

        let name_status = Command::new(git_executable)
            .args(["-C", root_string.as_str(), "config", "user.name", "test"])
            .status()
            .expect("git config user.name status");
        assert!(name_status.success(), "git config user.name should succeed");

        let email_status = Command::new(git_executable)
            .args([
                "-C",
                root_string.as_str(),
                "config",
                "user.email",
                "test@example.com",
            ])
            .status()
            .expect("git config user.email status");
        assert!(
            email_status.success(),
            "git config user.email should succeed"
        );

        std::fs::write(root.join("README.md"), "hello\n").expect("write README");

        let add_status = Command::new(git_executable)
            .args(["-C", root_string.as_str(), "add", "README.md"])
            .status()
            .expect("git add status");
        assert!(add_status.success(), "git add should succeed");

        let commit_status = Command::new(git_executable)
            .args([
                "-C",
                root_string.as_str(),
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                "init",
            ])
            .status()
            .expect("git commit status");
        assert!(commit_status.success(), "git commit should succeed");
    }

    fn worktree_test_config(root: &Path) -> LoongClawConfig {
        let mut config = LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config
    }

    #[test]
    fn prepare_delegate_workspace_root_creates_detached_worktree_under_repo_worktrees_dir() {
        let repo_root = crate::test_support::unique_temp_dir("delegate-worktree-create");
        init_git_repo(repo_root.as_path());
        let config = worktree_test_config(repo_root.as_path());

        let workspace_root = prepare_delegate_workspace_root(
            &config,
            "delegate:child-session",
            ConstrainedSubagentIsolation::Worktree,
        )
        .expect("prepare worktree")
        .expect("workspace root");

        assert!(workspace_root.ends_with("delegate-child-session"));
        assert!(workspace_root.join("README.md").exists());
    }

    #[test]
    fn cleanup_delegate_workspace_root_removes_clean_worktree_and_retains_dirty_one() {
        let repo_root = crate::test_support::unique_temp_dir("delegate-worktree-cleanup");
        init_git_repo(repo_root.as_path());
        let config = worktree_test_config(repo_root.as_path());

        let clean_root = prepare_delegate_workspace_root(
            &config,
            "delegate:clean-child",
            ConstrainedSubagentIsolation::Worktree,
        )
        .expect("prepare clean worktree")
        .expect("clean workspace root");
        let clean_execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            isolation: ConstrainedSubagentIsolation::Worktree,
            depth: 1,
            max_depth: 1,
            active_children: 0,
            max_active_children: 1,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["file.read".to_owned()],
            workspace_root: Some(clean_root.clone()),
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: None,
            agent_role: None,
        };
        let clean_cleanup = cleanup_delegate_workspace_root(&clean_execution)
            .expect("cleanup clean worktree")
            .expect("cleanup metadata");
        assert!(!clean_cleanup.retained);
        assert!(!clean_root.exists());

        let dirty_root = prepare_delegate_workspace_root(
            &config,
            "delegate:dirty-child",
            ConstrainedSubagentIsolation::Worktree,
        )
        .expect("prepare dirty worktree")
        .expect("dirty workspace root");
        std::fs::write(dirty_root.join("README.md"), "changed\n").expect("dirty worktree file");
        let dirty_execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            isolation: ConstrainedSubagentIsolation::Worktree,
            depth: 1,
            max_depth: 1,
            active_children: 0,
            max_active_children: 1,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["file.read".to_owned()],
            workspace_root: Some(dirty_root.clone()),
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: None,
            agent_role: None,
        };
        let dirty_cleanup = cleanup_delegate_workspace_root(&dirty_execution)
            .expect("cleanup dirty worktree")
            .expect("cleanup metadata");
        assert!(dirty_cleanup.retained);
        assert!(dirty_cleanup.dirty);
        assert!(dirty_root.exists());
    }
}
