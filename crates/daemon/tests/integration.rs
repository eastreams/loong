#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    unused_imports,
    dead_code,
    unsafe_code,
    clippy::disallowed_methods,
    clippy::undocumented_unsafe_blocks
)]
use std::time::Duration;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::Path,
    sync::MutexGuard,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use loongclaw_daemon::kernel::ConnectorCommand;
use loongclaw_daemon::kernel::{
    AuditEventKind, Capability, ExecutionRoute, HarnessKind, PluginBridgeKind, VerticalPackManifest,
};
use loongclaw_daemon::test_support::*;
use loongclaw_daemon::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::time::sleep;

pub struct MigrationEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl MigrationEnvironmentGuard {
    pub fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = lock_daemon_test_environment();
        let mut saved = Vec::new();
        let home_override = pairs
            .iter()
            .find_map(|(key, value)| (*key == "HOME").then_some(*value))
            .flatten()
            .map(std::path::PathBuf::from);
        let explicit_home_override = pairs
            .iter()
            .any(|(key, _)| *key == "LOONG_HOME" || *key == "LOONGCLAW_HOME");

        for (key, value) in pairs {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        if !explicit_home_override {
            saved.push(("LOONG_HOME".to_owned(), std::env::var_os("LOONG_HOME")));
            match home_override {
                Some(home) => unsafe {
                    std::env::set_var("LOONG_HOME", home.join(mvp::config::HOME_DIR_NAME))
                },
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

#[path = "integration/mod.rs"]
mod integration;
