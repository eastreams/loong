#![cfg(unix)]

use super::*;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static CHAT_CLI_TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = CHAT_CLI_TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loongclaw-chat-cli-{label}-{}-{nanos}-{counter}",
        std::process::id(),
    ))
}

fn prepend_path(dir: &Path) -> OsString {
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path_value = dir.as_os_str().to_os_string();
    if !original_path.is_empty() {
        path_value.push(OsStr::new(":"));
        path_value.push(original_path);
    }
    path_value
}

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

struct PermissionsResetGuard {
    path: PathBuf,
    permissions: std::fs::Permissions,
}

impl PermissionsResetGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            permissions: std::fs::metadata(path)
                .expect("metadata for permission reset")
                .permissions(),
        }
    }
}

impl Drop for PermissionsResetGuard {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
    }
}

struct ChatCliFixture {
    _lock: MutexGuard<'static, ()>,
    root: PathBuf,
    home_dir: PathBuf,
    bin_dir: PathBuf,
    onboard_log_path: PathBuf,
}

impl ChatCliFixture {
    fn new(label: &str) -> Self {
        let lock = lock_daemon_test_environment();
        let root = unique_temp_path(label);
        let home_dir = root.join("home");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&home_dir).expect("create fixture home");
        std::fs::create_dir_all(&bin_dir).expect("create fixture bin");
        Self {
            _lock: lock,
            home_dir,
            bin_dir,
            onboard_log_path: root.join("fake-onboard.log"),
            root,
        }
    }

    fn install_fake_loongclaw(&self, exit_code: i32) {
        let script_path = self.bin_dir.join("loongclaw");
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit {exit_code}\n",
            self.onboard_log_path.display()
        );
        std::fs::write(&script_path, script).expect("write fake loongclaw script");
        let mut permissions = std::fs::metadata(&script_path)
            .expect("fake loongclaw metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions)
            .expect("mark fake loongclaw executable");
    }

    fn run_chat_command(&self, config_path: Option<&Path>, stdin_bytes: Option<&[u8]>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_loongclaw"));
        command
            .arg("chat")
            .current_dir(&self.root)
            .env("HOME", &self.home_dir)
            .env("PATH", prepend_path(&self.bin_dir))
            .env_remove("LOONGCLAW_CONFIG_PATH")
            .env_remove("USERPROFILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(config_path) = config_path {
            command.arg("--config").arg(config_path);
        }

        let mut child = command.spawn().expect("spawn chat cli");
        if let Some(stdin_bytes) = stdin_bytes {
            child
                .stdin
                .as_mut()
                .expect("chat stdin")
                .write_all(stdin_bytes)
                .expect("write chat stdin");
        }
        drop(child.stdin.take());
        child.wait_with_output().expect("wait for chat cli output")
    }

    fn onboard_log(&self) -> String {
        std::fs::read_to_string(&self.onboard_log_path).unwrap_or_default()
    }
}

impl Drop for ChatCliFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn chat_without_config_runs_onboard_for_explicit_yes() {
    let fixture = ChatCliFixture::new("explicit-yes");
    fixture.install_fake_loongclaw(0);

    let output = fixture.run_chat_command(None, Some(b"yes\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "explicit yes should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains("Welcome to LoongClaw!"),
        "missing-config onboarding flow should greet the user: {stdout:?}"
    );
    assert!(
        fixture.onboard_log().contains("onboard"),
        "explicit yes should invoke `loongclaw onboard`: {:?}",
        fixture.onboard_log()
    );
}

#[test]
fn chat_without_config_treats_blank_line_as_decline() {
    let fixture = ChatCliFixture::new("blank-line");
    fixture.install_fake_loongclaw(0);

    let output = fixture.run_chat_command(None, Some(b"\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "blank input should exit cleanly, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        fixture.onboard_log().is_empty(),
        "blank input should not auto-run onboarding: {:?}",
        fixture.onboard_log()
    );
    assert!(
        stdout.contains("You can run 'loongclaw onboard' later to get started."),
        "blank input should leave a follow-up hint: {stdout:?}"
    );
}

#[test]
fn chat_without_config_treats_eof_as_decline() {
    let fixture = ChatCliFixture::new("eof");
    fixture.install_fake_loongclaw(0);

    let output = fixture.run_chat_command(None, None);
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "eof should exit cleanly, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        fixture.onboard_log().is_empty(),
        "eof should not auto-run onboarding: {:?}",
        fixture.onboard_log()
    );
    assert!(
        stdout.contains("You can run 'loongclaw onboard' later to get started."),
        "eof should still leave the follow-up hint: {stdout:?}"
    );
}

#[test]
fn chat_without_config_reports_onboard_failure() {
    let fixture = ChatCliFixture::new("onboard-failure");
    fixture.install_fake_loongclaw(7);

    let output = fixture.run_chat_command(None, Some(b"y\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "failing onboard should bubble up as a cli error, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stderr.contains("error: onboard exited with code Some(7)"),
        "stderr should surface the subprocess failure: {stderr:?}"
    );
}

#[test]
fn chat_without_config_surfaces_config_path_access_errors() {
    let fixture = ChatCliFixture::new("config-access-error");
    fixture.install_fake_loongclaw(0);

    let blocked_dir = fixture.root.join("blocked");
    std::fs::create_dir_all(&blocked_dir).expect("create blocked directory");
    let _reset_guard = PermissionsResetGuard::new(&blocked_dir);
    let mut permissions = std::fs::metadata(&blocked_dir)
        .expect("blocked directory metadata")
        .permissions();
    permissions.set_mode(0o000);
    std::fs::set_permissions(&blocked_dir, permissions).expect("lock blocked directory");
    let blocked_config = blocked_dir.join("loongclaw.toml");

    let output = fixture.run_chat_command(Some(&blocked_config), None);
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "config access errors should not fall into onboarding, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stderr.contains("failed to access config path"),
        "stderr should report the path access failure: {stderr:?}"
    );
    assert!(
        fixture.onboard_log().is_empty(),
        "path access errors should not invoke onboarding: {:?}",
        fixture.onboard_log()
    );
}
