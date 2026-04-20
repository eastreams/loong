#[path = "build_support/version_metadata.rs"]
mod version_metadata;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=LOONG_RELEASE_BUILD");
    println!("cargo:rerun-if-env-changed=LOONG_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=LOONG_GIT_SHA");
    println!("cargo:rerun-if-env-changed=LOONG_RELEASE_BUILD");
    println!("cargo:rerun-if-env-changed=LOONG_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=LOONG_GIT_SHA");

    let repo_root = repo_root();
    emit_git_rerun_hints(&repo_root);

    let release_build_env = env::var("LOONG_RELEASE_BUILD")
        .ok()
        .or_else(|| env::var("LOONG_RELEASE_BUILD").ok());
    let channel_env = env::var("LOONG_BUILD_CHANNEL")
        .ok()
        .or_else(|| env::var("LOONG_BUILD_CHANNEL").ok());
    let sha_env = env::var("LOONG_GIT_SHA")
        .ok()
        .or_else(|| env::var("LOONG_GIT_SHA").ok());
    let git_branch = git_output(&repo_root, &["branch", "--show-current"]);
    let git_sha = git_output(&repo_root, &["rev-parse", "--short=7", "HEAD"]);

    let metadata = version_metadata::resolve_build_metadata(
        release_build_env.as_deref(),
        channel_env.as_deref(),
        sha_env.as_deref(),
        git_branch.as_deref(),
        git_sha.as_deref(),
    );

    emit_rustc_env(
        "LOONG_RELEASE_BUILD",
        if metadata.release_build { "1" } else { "" },
    );
    emit_rustc_env(
        "LOONG_RELEASE_BUILD",
        if metadata.release_build { "1" } else { "" },
    );
    emit_rustc_env(
        "LOONG_BUILD_CHANNEL",
        metadata.channel.as_deref().unwrap_or(""),
    );
    emit_rustc_env(
        "LOONG_BUILD_CHANNEL",
        metadata.channel.as_deref().unwrap_or(""),
    );
    emit_rustc_env("LOONG_GIT_SHA", metadata.short_sha.as_deref().unwrap_or(""));
    emit_rustc_env("LOONG_GIT_SHA", metadata.short_sha.as_deref().unwrap_or(""));
}

fn repo_root() -> PathBuf {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest_dir)
}

fn emit_git_rerun_hints(repo_root: &Path) {
    let symbolic_head_ref = git_output(repo_root, &["rev-parse", "--symbolic-full-name", "HEAD"])
        .filter(|head_ref| head_ref != "HEAD");

    for target in version_metadata::git_rerun_hint_targets(symbolic_head_ref.as_deref()) {
        if let Some(path) = git_output(repo_root, &["rev-parse", "--git-path", target.as_str()]) {
            let rerun_path = version_metadata::resolve_git_rerun_hint_path(repo_root, &path);
            println!("cargo:rerun-if-changed={}", rerun_path.display());
        }
    }
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn emit_rustc_env(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}
