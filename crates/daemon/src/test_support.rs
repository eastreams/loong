use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::mvp;
use crate::{Capability, OperationSpec};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;

pub use crate::SecurityScanProfile;

static DAEMON_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn lock_daemon_test_environment() -> MutexGuard<'static, ()> {
    DAEMON_TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn set_test_env_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so no concurrent env readers or writers
    // observe racy updates while these helpers are active.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_test_env_var(key: impl AsRef<OsStr>) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so removals are coordinated with all
    // other daemon-side env mutation helpers.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::remove_var(key);
    }
}

pub struct ScopedEnv {
    saved: Vec<(String, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl ScopedEnv {
    pub fn new() -> Self {
        let lock = lock_daemon_test_environment();
        let saved = Vec::new();
        Self { saved, _lock: lock }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl AsRef<OsStr>) {
        let key = key.into();
        self.capture_original(&key);
        set_test_env_var(&key, value);
    }

    pub fn remove(&mut self, key: impl Into<String>) {
        let key = key.into();
        self.capture_original(&key);
        remove_test_env_var(&key);
    }

    fn capture_original(&mut self, key: &str) {
        let already_saved = self.saved.iter().any(|(saved_key, _)| saved_key == key);
        if already_saved {
            return;
        }

        let original_value = std::env::var_os(key);
        let saved_key = key.to_owned();
        self.saved.push((saved_key, original_value));
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        while let Some((key, original_value)) = self.saved.pop() {
            match original_value {
                Some(value) => set_test_env_var(&key, value),
                None => remove_test_env_var(&key),
            }
        }
    }
}

pub fn catalog_entry(raw: &str) -> mvp::channel::ChannelCatalogEntry {
    mvp::channel::resolve_channel_catalog_entry(raw).expect("channel catalog entry")
}

pub fn catalog_command_family(raw: &str) -> mvp::channel::ChannelCatalogCommandFamilyDescriptor {
    mvp::channel::resolve_channel_catalog_command_family_descriptor(raw)
        .expect("channel catalog command family")
}

pub fn channel_send_command(raw: &str) -> &'static str {
    catalog_command_family(raw).send.command
}

pub fn channel_serve_command(raw: &str) -> &'static str {
    catalog_command_family(raw).serve.command
}

pub fn channel_capability_ids(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .capabilities
        .into_iter()
        .map(|capability| capability.as_str())
        .collect()
}

pub fn channel_supported_target_kinds(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .supported_target_kinds
        .into_iter()
        .map(|kind| kind.as_str())
        .collect()
}

pub fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
    OperationSpec::ToolCore {
        tool_name: tool_name.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload,
        core: None,
    }
}

pub fn write_temp_risk_profile(path: &Path, body: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("temp risk profile path should have parent directory"),
    )
    .expect("create temp risk profile directory");
    fs::write(path, body).expect("write temp risk profile");
}

pub fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
    use crate::security_scan_profile_message;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&security_scan_profile_message(profile));
    let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
    (public_key_base64, signature_base64)
}

#[cfg(test)]
mod tests {
    use super::ScopedEnv;

    #[test]
    fn scoped_env_remove_restores_original_value() {
        let key = "LOONGCLAW_SCOPED_ENV_REMOVE_TEST_KEY";
        let sentinel_value = "scoped-env-sentinel";
        let mut env = ScopedEnv::new();
        let original_value = std::env::var_os(key);

        env.set(key, sentinel_value);
        env.remove(key);

        assert!(
            std::env::var_os(key).is_none(),
            "ScopedEnv::remove should clear the environment variable while the guard is alive"
        );

        drop(env);

        assert_eq!(
            std::env::var_os(key),
            original_value,
            "ScopedEnv should restore the original environment value when dropped"
        );
    }
}
