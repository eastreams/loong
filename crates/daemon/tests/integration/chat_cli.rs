#![cfg(unix)]

use super::latest_selector_process_support::LatestSelectorCliFixture;
use super::*;
use std::ffi::OsStr;
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

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn invoked_chat_cli_command_name() -> &'static str {
    mvp::config::detect_invoked_cli_command_name_from_arg0(Some(OsStr::new(env!(
        "CARGO_BIN_EXE_loongclaw"
    ))))
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
    onboard_binary_path: PathBuf,
    onboard_log_path: PathBuf,
}

impl ChatCliFixture {
    fn new(label: &str) -> Self {
        let lock = lock_daemon_test_environment();
        let root = unique_temp_path(label);
        let home_dir = root.join("home");
        let bin_dir = root.join("bin");
        let onboard_binary_path = bin_dir.join("fake-onboard");
        std::fs::create_dir_all(&home_dir).expect("create fixture home");
        std::fs::create_dir_all(&bin_dir).expect("create fixture bin");
        Self {
            _lock: lock,
            home_dir,
            onboard_binary_path,
            onboard_log_path: root.join("fake-onboard.log"),
            root,
        }
    }

    fn install_fake_onboard(&self, exit_code: i32) {
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit {exit_code}\n",
            self.onboard_log_path.display()
        );
        std::fs::write(&self.onboard_binary_path, script).expect("write fake loongclaw script");
        let mut permissions = std::fs::metadata(&self.onboard_binary_path)
            .expect("fake loongclaw metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&self.onboard_binary_path, permissions)
            .expect("mark fake loongclaw executable");
    }

    fn run_chat_command(&self, config_path: Option<&Path>, stdin_bytes: Option<&[u8]>) -> Output {
        self.run_chat_command_with_fake_onboard(config_path, stdin_bytes, None)
    }

    fn run_chat_command_with_fake_onboard(
        &self,
        config_path: Option<&Path>,
        stdin_bytes: Option<&[u8]>,
        fake_onboard_exit_code: Option<i32>,
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_loongclaw"));
        command
            .arg("chat")
            .current_dir(&self.root)
            .env("HOME", &self.home_dir)
            .env_remove("LOONGCLAW_CONFIG_PATH")
            .env_remove("USERPROFILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(config_path) = config_path {
            command.arg("--config").arg(config_path);
        }
        if let Some(exit_code) = fake_onboard_exit_code {
            self.install_fake_onboard(exit_code);
            command.env(
                "LOONGCLAW_TEST_ONBOARD_EXECUTABLE",
                &self.onboard_binary_path,
            );
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
    let output = fixture.run_chat_command_with_fake_onboard(None, Some(b"yes\n"), Some(0));
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
        "explicit yes should invoke `{} onboard`: {:?}",
        super::active_cli_command_name(),
        fixture.onboard_log()
    );
}

#[test]
fn chat_without_config_runs_onboard_for_default_enter() {
    let fixture = ChatCliFixture::new("default-enter");
    let output = fixture.run_chat_command_with_fake_onboard(None, Some(b"\n"), Some(0));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "default enter should now accept onboarding, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        fixture.onboard_log().contains("onboard"),
        "default enter should invoke `{} onboard`: {:?}",
        super::active_cli_command_name(),
        fixture.onboard_log()
    );
}

#[test]
fn chat_without_config_forwards_explicit_config_path_to_onboard() {
    let fixture = ChatCliFixture::new("explicit-config");
    let explicit_config = fixture.root.join("custom-config.toml");

    let output =
        fixture.run_chat_command_with_fake_onboard(Some(&explicit_config), Some(b"y\n"), Some(0));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let onboard_log = fixture.onboard_log();

    assert!(
        output.status.success(),
        "explicit config path should still succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        onboard_log.contains("onboard --output"),
        "explicit config path should be forwarded to onboarding output args: {onboard_log:?}"
    );
    assert!(
        onboard_log.contains(&explicit_config.display().to_string()),
        "explicit config path should be passed through to onboarding: {onboard_log:?}"
    );
}

#[test]
fn chat_without_config_decline_hint_preserves_explicit_config_path() {
    let fixture = ChatCliFixture::new("decline-explicit-config");
    let explicit_config = fixture.root.join("custom-config.toml");

    let output = fixture.run_chat_command(Some(&explicit_config), Some(b"n\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let expected_hint = format!(
        "You can run '{} onboard --output {}' later to get started.",
        invoked_chat_cli_command_name(),
        explicit_config.display()
    );
    let compacted_stdout = stdout.split_whitespace().collect::<String>();
    let compacted_expected_hint = expected_hint.split_whitespace().collect::<String>();

    assert!(
        output.status.success(),
        "declining with an explicit config path should still exit cleanly, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        fixture.onboard_log().is_empty(),
        "declining should not spawn onboarding: {:?}",
        fixture.onboard_log()
    );
    assert!(
        compacted_stdout.contains(&compacted_expected_hint),
        "decline hint should preserve the explicit config path: {stdout:?}"
    );
}

#[test]
fn chat_without_config_treats_explicit_no_as_decline() {
    let fixture = ChatCliFixture::new("explicit-no");

    let output = fixture.run_chat_command(None, Some(b"n\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let expected_hint = format!(
        "You can run '{} onboard' later to get started.",
        invoked_chat_cli_command_name()
    );
    let compacted_stdout = stdout.split_whitespace().collect::<String>();
    let compacted_expected_hint = expected_hint.split_whitespace().collect::<String>();

    assert!(
        output.status.success(),
        "explicit no should exit cleanly, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        fixture.onboard_log().is_empty(),
        "explicit no should not auto-run onboarding: {:?}",
        fixture.onboard_log()
    );
    assert!(
        compacted_stdout.contains(&compacted_expected_hint),
        "explicit no should leave a follow-up hint: {stdout:?}"
    );
}

#[test]
fn chat_without_config_treats_eof_as_decline() {
    let fixture = ChatCliFixture::new("eof");

    let output = fixture.run_chat_command(None, None);
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let expected_hint = format!(
        "You can run '{} onboard' later to get started.",
        invoked_chat_cli_command_name()
    );
    let compacted_stdout = stdout.split_whitespace().collect::<String>();
    let compacted_expected_hint = expected_hint.split_whitespace().collect::<String>();

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
        compacted_stdout.contains(&compacted_expected_hint),
        "eof should still leave the follow-up hint: {stdout:?}"
    );
}

#[test]
fn chat_without_config_reports_onboard_failure() {
    let fixture = ChatCliFixture::new("onboard-failure");
    let output = fixture.run_chat_command_with_fake_onboard(None, Some(b"y\n"), Some(7));
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

#[test]
fn chat_cli_latest_session_selector_process_uses_selected_root_session_history() {
    let fixture = LatestSelectorCliFixture::new("chat-latest-selector-process");
    fixture.write_config_with(|_| {});

    fixture.create_root_session("root-old");
    fixture.append_session_turn("root-old", "user", "old root turn");
    fixture.set_session_updated_at("root-old", 100);
    fixture.set_turn_timestamps("root-old", 100);

    fixture.create_root_session("root-new");
    fixture.append_session_turn("root-new", "user", "selected user turn");
    fixture.append_session_turn("root-new", "assistant", "selected assistant turn");
    fixture.set_session_updated_at("root-new", 200);
    fixture.set_turn_timestamps("root-new", 200);

    fixture.create_delegate_child_session("delegate-child", "root-new");
    fixture.append_session_turn("delegate-child", "assistant", "delegate child turn");
    fixture.set_session_updated_at("delegate-child", 400);
    fixture.set_turn_timestamps("delegate-child", 400);

    fixture.create_root_session("root-archived");
    fixture.append_session_turn("root-archived", "assistant", "archived root turn");
    fixture.set_session_updated_at("root-archived", 500);
    fixture.set_turn_timestamps("root-archived", 500);
    fixture.archive_session("root-archived", 600);

    let output = fixture.run_process(&["chat", "--session", "latest"], Some(b"/history\n/exit\n"));
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "chat latest selector should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains("- session: root-new"),
        "chat startup should surface the resolved latest session id: {stdout:?}"
    );
    assert!(
        stdout.contains("selected user turn"),
        "history output should include the selected latest root user turn: {stdout:?}"
    );
    assert!(
        stdout.contains("selected assistant turn"),
        "history output should include the selected latest root assistant turn: {stdout:?}"
    );
    assert!(
        !stdout.contains("old root turn"),
        "older root history should not appear in the latest session output: {stdout:?}"
    );
    assert!(
        !stdout.contains("delegate child turn"),
        "delegate child history should not appear in the latest root output: {stdout:?}"
    );
    assert!(
        !stdout.contains("archived root turn"),
        "archived root history should not appear in the latest root output: {stdout:?}"
    );
}

#[test]
fn chat_cli_latest_session_selector_process_rejects_missing_resumable_root() {
    let fixture = LatestSelectorCliFixture::new("chat-latest-selector-empty");
    fixture.write_config_with(|_| {});

    let output = fixture.run_process(&["chat", "--session", "latest"], None);
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing latest root session should fail before chat starts, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stderr.contains("latest"),
        "chat error output should mention the latest selector: {stderr:?}"
    );
    assert!(
        stderr.contains("resumable root session"),
        "chat error output should explain the missing latest root session: {stderr:?}"
    );
}
