#![allow(unused_imports)]

pub use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
pub use clap::{CommandFactory, Parser};
pub use loong_daemon::kernel::ConnectorCommand;
pub use loong_daemon::kernel::{
    AuditEventKind, Capability, ExecutionRoute, HarnessKind, PluginBridgeKind, VerticalPackManifest,
};
pub use loong_daemon::test_support::*;
pub use loong_daemon::*;
pub use serde_json::{Value, json};
pub use sha2::{Digest, Sha256};
pub use std::time::Duration;
pub use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};
pub use tokio::time::sleep;

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(debug_assertions)]
const CLI_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
#[cfg(not(debug_assertions))]
const CLI_STACK_SIZE_BYTES: usize = 8 * 1024 * 1024;

static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn home_override_from_pairs(pairs: &[(&str, Option<&str>)]) -> Option<PathBuf> {
    for (key, value) in pairs {
        if *key != "HOME" {
            continue;
        }

        let value = *value;
        let value = value?;
        let home_path = PathBuf::from(value);
        return Some(home_path);
    }

    None
}

fn has_explicit_loong_home_override(pairs: &[(&str, Option<&str>)]) -> bool {
    for (key, _) in pairs {
        if *key == "LOONG_HOME" {
            return true;
        }
    }

    false
}

fn save_current_env(saved: &mut Vec<(String, Option<OsString>)>, key: &str) {
    let current_value = std::env::var_os(key);
    let saved_entry = (key.to_owned(), current_value);
    saved.push(saved_entry);
}

fn set_or_remove_env(key: &str, value: Option<&str>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

pub(crate) struct MigrationEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl MigrationEnvironmentGuard {
    pub(crate) fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = lock_daemon_test_environment();
        let mut saved = Vec::new();
        let home_override = home_override_from_pairs(pairs);
        let explicit_home_override = has_explicit_loong_home_override(pairs);

        for (key, value) in pairs {
            let key = *key;
            let value = *value;
            save_current_env(&mut saved, key);
            set_or_remove_env(key, value);
        }

        if !explicit_home_override {
            save_current_env(&mut saved, "LOONG_HOME");
            match home_override {
                Some(home) => {
                    let loong_home = home.join(mvp::config::HOME_DIR_NAME);
                    unsafe { std::env::set_var("LOONG_HOME", loong_home) };
                }
                None => unsafe { std::env::remove_var("LOONG_HOME") },
            }
        }

        Self { _lock: lock, saved }
    }
}

impl Drop for MigrationEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
    }
}

fn with_cli_stack<T, F>(thread_name: &str, operation: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let thread_builder = std::thread::Builder::new();
    let thread_builder = thread_builder.name(thread_name.to_owned());
    let thread_builder = thread_builder.stack_size(CLI_STACK_SIZE_BYTES);
    let join_handle = thread_builder
        .spawn(operation)
        .expect("spawn CLI stack thread");
    match join_handle.join() {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn collect_cli_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
    let mut owned_args = Vec::with_capacity(N);

    for arg in args {
        let owned_arg = OsString::from(arg);
        owned_args.push(owned_arg);
    }

    owned_args
}

pub(crate) fn try_parse_cli<const N: usize>(args: [&str; N]) -> Result<Cli, clap::Error> {
    let owned_args = collect_cli_args(args);
    with_cli_stack("daemon-test-cli-parse", move || {
        Cli::try_parse_from(owned_args)
    })
}

fn collect_subcommand_path<const N: usize>(subcommand_path: [&str; N]) -> Vec<String> {
    let mut owned_path = Vec::with_capacity(N);

    for subcommand in subcommand_path {
        let owned_subcommand = subcommand.to_owned();
        owned_path.push(owned_subcommand);
    }

    owned_path
}

pub(crate) fn render_cli_help<const N: usize>(subcommand_path: [&str; N]) -> String {
    let owned_path = collect_subcommand_path(subcommand_path);
    with_cli_stack("daemon-test-cli-help", move || {
        let mut command = Cli::command();
        let mut current = &mut command;

        for subcommand in owned_path {
            let lookup_name = subcommand.as_str();
            let next = current.find_subcommand_mut(lookup_name);
            let next = next.unwrap_or_else(|| panic!("missing CLI subcommand `{subcommand}`"));
            current = next;
        }

        let mut rendered = Vec::new();
        current
            .write_long_help(&mut rendered)
            .expect("render CLI help");
        String::from_utf8(rendered).expect("help should be utf8")
    })
}

pub(crate) fn cli_command_name() -> String {
    with_cli_stack("daemon-test-cli-command-name", || {
        let command = Cli::command();
        command.get_name().to_owned()
    })
}

pub(crate) fn unique_temp_dir(label: &str) -> PathBuf {
    let now = SystemTime::now();
    let elapsed = now
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch");
    let nanos = elapsed.as_nanos();
    let counter = TEST_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir();
    let canonical_temp_dir = dunce::canonicalize(&temp_dir).unwrap_or(temp_dir);
    let process_id = std::process::id();
    let directory_name = format!("loong-integration-{label}-{process_id}-{nanos}-{counter}");
    canonical_temp_dir.join(directory_name)
}

#[cfg(unix)]
pub(crate) fn integration_permission_test_running_as_root() -> bool {
    let status = std::fs::read_to_string("/proc/self/status");
    let Ok(status) = status else {
        return false;
    };

    let uid_line = status.lines().find(|line| line.starts_with("Uid:"));
    let Some(uid_line) = uid_line else {
        return false;
    };

    let fields = uid_line.split_whitespace();
    let mut fields = fields.skip(1);
    let real_uid = fields.next();
    let Some(real_uid) = real_uid else {
        return false;
    };

    real_uid == "0"
}

#[cfg(not(unix))]
pub(crate) fn integration_permission_test_running_as_root() -> bool {
    false
}

pub(crate) fn validation_diagnostic_with_severity(
    severity: &str,
    code: &str,
) -> mvp::config::ConfigValidationDiagnostic {
    let problem_type = format!("urn:loong:problem:{code}");
    let title_key = format!("{code}.title");
    let title = code.to_owned();
    let message_key = code.to_owned();
    let message_variables = BTreeMap::new();
    let message = code.to_owned();

    mvp::config::ConfigValidationDiagnostic {
        severity: severity.to_owned(),
        code: code.to_owned(),
        problem_type,
        title_key,
        title,
        message_key,
        message_locale: "en".to_owned(),
        message_variables,
        field_path: "active_provider".to_owned(),
        inline_field_path: "providers".to_owned(),
        example_env_name: String::new(),
        suggested_env_name: None,
        message,
    }
}
