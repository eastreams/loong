use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

const TOOL_LEASE_TTL_SECONDS: u64 = 300;
const TOOL_LEASE_SECRET_BYTES: usize = 32;
const TOOL_LEASE_SECRET_FILE_NAME: &str = "tool-lease-secret.hex";
const TOOL_LEASE_SECRET_PUBLICATION_RETRY_ATTEMPTS: usize = 12;
const TOOL_LEASE_SECRET_PUBLICATION_RETRY_DELAY_MILLIS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolLeaseClaims {
    tool_id: String,
    catalog_digest: String,
    expires_at_unix: u64,
    token_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ToolLeaseBinding {
    token_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
}

pub(crate) fn issue_tool_lease(
    tool_id: &str,
    payload: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let binding = extract_tool_lease_binding(payload);
    let catalog_digest = tool_catalog_digest();
    let expires_at_unix = now_unix_seconds().saturating_add(TOOL_LEASE_TTL_SECONDS);
    let claims = ToolLeaseClaims {
        tool_id: tool_id.to_owned(),
        catalog_digest,
        expires_at_unix,
        token_id: binding.token_id,
        session_id: binding.session_id,
        turn_id: binding.turn_id,
    };
    let claims_bytes = serde_json::to_vec(&claims)
        .map_err(|error| format!("tool_lease_claims_serialize_failed: {error}"))?;
    let encoded_claims = URL_SAFE_NO_PAD.encode(claims_bytes);
    let signature = sign_tool_lease(encoded_claims.as_str())?;
    let lease = format!("{encoded_claims}.{signature}");
    Ok(lease)
}

pub(crate) fn validate_tool_lease(
    expected_tool_id: &str,
    lease: &str,
    payload: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let split = lease.split_once('.');
    let Some((encoded_claims, signature)) = split else {
        return Err("invalid_tool_lease: malformed lease".to_owned());
    };

    let signatures_match = tool_lease_signature_matches(encoded_claims, signature)?;
    if !signatures_match {
        return Err("invalid_tool_lease: signature mismatch".to_owned());
    }

    let claims_bytes = URL_SAFE_NO_PAD
        .decode(encoded_claims)
        .map_err(|error| format!("invalid_tool_lease: claims decode failed: {error}"))?;
    let claims: ToolLeaseClaims = serde_json::from_slice(&claims_bytes)
        .map_err(|error| format!("invalid_tool_lease: claims parse failed: {error}"))?;

    let tool_matches = claims.tool_id == expected_tool_id;
    if !tool_matches {
        return Err("invalid_tool_lease: tool mismatch".to_owned());
    }

    let catalog_digest = tool_catalog_digest();
    let catalog_matches = claims.catalog_digest == catalog_digest;
    if !catalog_matches {
        return Err("invalid_tool_lease: catalog mismatch".to_owned());
    }

    let now_unix = now_unix_seconds();
    let lease_is_expired = claims.expires_at_unix <= now_unix;
    if lease_is_expired {
        return Err("invalid_tool_lease: expired lease".to_owned());
    }

    let binding = extract_tool_lease_binding(payload);
    let token_matches = claims.token_id.is_none() || claims.token_id == binding.token_id;
    if !token_matches {
        return Err("invalid_tool_lease: token mismatch".to_owned());
    }

    let session_matches = claims.session_id.is_none() || claims.session_id == binding.session_id;
    if !session_matches {
        return Err("invalid_tool_lease: session mismatch".to_owned());
    }

    let turn_matches = claims.turn_id.is_none() || claims.turn_id == binding.turn_id;
    if !turn_matches {
        return Err("invalid_tool_lease: turn mismatch".to_owned());
    }

    Ok(())
}

fn extract_tool_lease_binding(payload: &serde_json::Map<String, Value>) -> ToolLeaseBinding {
    let token_id = payload
        .get(super::TOOL_LEASE_TOKEN_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let session_id = payload
        .get(super::TOOL_LEASE_SESSION_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let turn_id = payload
        .get(super::TOOL_LEASE_TURN_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    ToolLeaseBinding {
        token_id,
        session_id,
        turn_id,
    }
}

fn sign_tool_lease(encoded_claims: &str) -> Result<String, String> {
    let secret = tool_lease_secret()?;
    Ok(sign_tool_lease_with_secret(encoded_claims, secret.as_str()))
}

fn sign_tool_lease_with_secret(encoded_claims: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(encoded_claims.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

fn tool_lease_signature_matches(encoded_claims: &str, signature: &str) -> Result<bool, String> {
    let expected_signature = sign_tool_lease(encoded_claims)?;
    let primary_match =
        crate::crypto::timing_safe_eq(expected_signature.as_bytes(), signature.as_bytes());
    if primary_match {
        return Ok(true);
    }

    #[cfg(test)]
    {
        for cached_secret in cached_tool_lease_secrets_for_tests() {
            let cached_signature =
                sign_tool_lease_with_secret(encoded_claims, cached_secret.as_str());
            let cached_match =
                crate::crypto::timing_safe_eq(cached_signature.as_bytes(), signature.as_bytes());
            if cached_match {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
fn cached_tool_lease_secrets_for_tests() -> Vec<String> {
    let cache = tool_lease_secret_cache();
    let guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut secrets = Vec::new();
    for secret in guard.values() {
        if secrets.iter().any(|cached| cached == secret) {
            continue;
        }
        secrets.push(secret.clone());
    }
    secrets
}

pub(crate) fn tool_catalog_digest() -> String {
    let digest = super::catalog::stable_tool_catalog_digest();
    digest.to_owned()
}

fn tool_lease_secret() -> Result<String, String> {
    let secret_path = default_tool_lease_secret_path();
    let cached_secret = cached_tool_lease_secret(secret_path.as_path());
    if let Some(cached_secret) = cached_secret {
        return Ok(cached_secret);
    }

    let loaded_secret = load_or_create_tool_lease_secret(secret_path.as_path())?;
    cache_tool_lease_secret(secret_path, loaded_secret.clone());
    Ok(loaded_secret)
}

fn default_tool_lease_secret_path() -> PathBuf {
    let loongclaw_home = crate::config::default_loongclaw_home();
    loongclaw_home.join(TOOL_LEASE_SECRET_FILE_NAME)
}

fn load_or_create_tool_lease_secret(secret_path: &Path) -> Result<String, String> {
    let existing_secret = read_tool_lease_secret_file(secret_path)?;
    if let Some(existing_secret) = existing_secret {
        return Ok(existing_secret);
    }

    ensure_tool_lease_secret_parent_dir(secret_path)?;

    let generated_secret = generate_tool_lease_secret();
    let create_result = write_tool_lease_secret_if_missing(secret_path, generated_secret.as_str());

    match create_result {
        Ok(()) => Ok(generated_secret),
        Err(CreateToolLeaseSecretError::AlreadyExists) => {
            read_tool_lease_secret_after_competitor_publish(secret_path)
        }
        Err(CreateToolLeaseSecretError::Io(error)) => {
            let message = format!(
                "tool_lease_authority_unavailable: failed to persist secret at {}: {error}",
                secret_path.display()
            );
            Err(message)
        }
    }
}

fn read_tool_lease_secret_after_competitor_publish(secret_path: &Path) -> Result<String, String> {
    let retry_attempts = TOOL_LEASE_SECRET_PUBLICATION_RETRY_ATTEMPTS;
    let retry_delay = Duration::from_millis(TOOL_LEASE_SECRET_PUBLICATION_RETRY_DELAY_MILLIS);
    let mut attempt_index = 0usize;

    while attempt_index < retry_attempts {
        let existing_secret = read_tool_lease_secret_file(secret_path)?;
        if let Some(existing_secret) = existing_secret {
            return Ok(existing_secret);
        }

        attempt_index += 1;

        let has_more_attempts = attempt_index < retry_attempts;
        if !has_more_attempts {
            break;
        }

        thread::park_timeout(retry_delay);
    }

    let message = format!(
        "tool_lease_authority_unavailable: secret file appeared without readable contents at {}",
        secret_path.display()
    );
    Err(message)
}

fn ensure_tool_lease_secret_parent_dir(secret_path: &Path) -> Result<(), String> {
    let parent = secret_path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };

    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "tool_lease_authority_unavailable: failed to create secret directory {}: {error}",
            parent.display()
        )
    })
}

fn read_tool_lease_secret_file(secret_path: &Path) -> Result<Option<String>, String> {
    let raw_secret = match fs::read_to_string(secret_path) {
        Ok(raw_secret) => raw_secret,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            let message = format!(
                "tool_lease_authority_unavailable: failed to read secret file {}: {error}",
                secret_path.display()
            );
            return Err(message);
        }
    };

    let trimmed_secret = raw_secret.trim();
    if trimmed_secret.is_empty() {
        let message = format!(
            "tool_lease_authority_unavailable: secret file {} is empty",
            secret_path.display()
        );
        return Err(message);
    }

    let decoded_secret = hex::decode(trimmed_secret).map_err(|error| {
        format!(
            "tool_lease_authority_unavailable: secret file {} is not valid hex: {error}",
            secret_path.display()
        )
    })?;

    let secret_length = decoded_secret.len();
    let has_expected_length = secret_length == TOOL_LEASE_SECRET_BYTES;
    if !has_expected_length {
        let message = format!(
            "tool_lease_authority_unavailable: secret file {} has {} bytes; expected {}",
            secret_path.display(),
            secret_length,
            TOOL_LEASE_SECRET_BYTES
        );
        return Err(message);
    }

    let normalized_secret = trimmed_secret.to_owned();
    Ok(Some(normalized_secret))
}

fn generate_tool_lease_secret() -> String {
    let secret_bytes = rand::random::<[u8; TOOL_LEASE_SECRET_BYTES]>();
    hex::encode(secret_bytes)
}

enum CreateToolLeaseSecretError {
    AlreadyExists,
    Io(std::io::Error),
}

fn write_tool_lease_secret_if_missing(
    secret_path: &Path,
    secret: &str,
) -> Result<(), CreateToolLeaseSecretError> {
    let parent = secret_path.parent().unwrap_or_else(|| Path::new("."));
    let staged_file_result = tempfile::NamedTempFile::new_in(parent);
    let mut staged_file = match staged_file_result {
        Ok(staged_file) => staged_file,
        Err(error) => return Err(CreateToolLeaseSecretError::Io(error)),
    };

    let write_result = writeln!(staged_file, "{secret}");
    if let Err(error) = write_result {
        return Err(CreateToolLeaseSecretError::Io(error));
    }

    let sync_result = staged_file.as_file_mut().sync_all();
    if let Err(error) = sync_result {
        return Err(CreateToolLeaseSecretError::Io(error));
    }

    let persist_result = staged_file.persist_noclobber(secret_path);
    match persist_result {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(CreateToolLeaseSecretError::AlreadyExists)
        }
        Err(error) => Err(CreateToolLeaseSecretError::Io(error.error)),
    }
}

fn cached_tool_lease_secret(secret_path: &Path) -> Option<String> {
    let cache = tool_lease_secret_cache();
    let guard = cache.lock();
    let guard = match guard {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.get(secret_path).cloned()
}

fn cache_tool_lease_secret(secret_path: PathBuf, secret: String) {
    let cache = tool_lease_secret_cache();
    let guard = cache.lock();
    let mut guard = match guard {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.insert(secret_path, secret);
}

fn tool_lease_secret_cache() -> &'static Mutex<HashMap<PathBuf, String>> {
    static TOOL_LEASE_SECRET_CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    TOOL_LEASE_SECRET_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_unix_seconds() -> u64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    duration.as_secs()
}

#[cfg(test)]
pub(crate) fn clear_tool_lease_secret_cache_for_tests() {
    let cache = tool_lease_secret_cache();
    let guard = cache.lock();
    let mut guard = match guard {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::clear_tool_lease_secret_cache_for_tests;
    use super::default_tool_lease_secret_path;
    use super::generate_tool_lease_secret;
    use super::issue_tool_lease;
    use super::read_tool_lease_secret_after_competitor_publish;
    use super::read_tool_lease_secret_file;
    use super::validate_tool_lease;
    use crate::test_support::ScopedLoongClawHome;

    fn scoped_tool_lease_home(prefix: &str) -> ScopedLoongClawHome {
        ScopedLoongClawHome::new(prefix)
    }

    #[test]
    fn issue_tool_lease_persists_secret_under_loong_home() {
        let _home = scoped_tool_lease_home("loongclaw-tool-lease-home");
        let payload = serde_json::Map::new();

        let lease = issue_tool_lease("file.read", &payload).expect("lease");
        let secret_path = default_tool_lease_secret_path();
        let persisted_secret =
            read_tool_lease_secret_file(secret_path.as_path()).expect("persisted secret");

        assert!(!lease.is_empty());
        assert!(secret_path.exists());
        assert!(persisted_secret.is_some());
    }

    #[test]
    fn issued_tool_lease_survives_authority_cache_reset() {
        let _home = scoped_tool_lease_home("loongclaw-tool-lease-cache-home");
        let payload = serde_json::Map::new();

        let lease = issue_tool_lease("file.read", &payload).expect("lease");

        clear_tool_lease_secret_cache_for_tests();

        let validation_result = validate_tool_lease("file.read", &lease, &payload);

        validation_result.expect("lease should survive authority reload");
    }

    #[test]
    fn issue_tool_lease_parallel_first_use_keeps_secret_readable() {
        use std::sync::Arc;
        use std::sync::Barrier;

        let home = scoped_tool_lease_home("loongclaw-tool-lease-parallel-home");
        let home_path = home.path().to_path_buf();
        let thread_count = 8;
        let barrier = Arc::new(Barrier::new(thread_count));
        let mut handles = Vec::new();

        for _ in 0..thread_count {
            let barrier = Arc::clone(&barrier);
            let home_path = home_path.clone();
            let handle = std::thread::spawn(move || {
                let _thread_home =
                    crate::test_support::ScopedLoongClawHome::from_existing(home_path);
                let payload = serde_json::Map::new();
                barrier.wait();
                issue_tool_lease("file.read", &payload)
            });
            handles.push(handle);
        }

        for handle in handles {
            let join_result = handle.join().expect("join tool lease thread");
            join_result.expect("lease should issue without exposing an empty secret file");
        }

        let secret_path = default_tool_lease_secret_path();
        let persisted_secret =
            read_tool_lease_secret_file(secret_path.as_path()).expect("persisted secret");

        assert!(persisted_secret.is_some());

        drop(home);
    }

    #[test]
    fn read_tool_lease_secret_after_competitor_publish_waits_for_visible_secret() {
        let _home = scoped_tool_lease_home("loongclaw-tool-lease-visibility-home");
        let secret_path = default_tool_lease_secret_path();
        let parent_dir = secret_path.parent().expect("secret parent").to_path_buf();
        std::fs::create_dir_all(&parent_dir).expect("create secret parent");

        let expected_secret = generate_tool_lease_secret();
        let publisher_path = secret_path.clone();
        let publisher_secret = expected_secret.clone();

        let publisher = std::thread::spawn(move || {
            let publish_delay = std::time::Duration::from_millis(10);
            std::thread::park_timeout(publish_delay);
            let secret_body = format!("{publisher_secret}\n");
            std::fs::write(&publisher_path, secret_body).expect("publish secret file");
        });

        let observed_secret =
            read_tool_lease_secret_after_competitor_publish(secret_path.as_path())
                .expect("wait for visible secret");

        publisher.join().expect("join publisher thread");

        assert_eq!(observed_secret, expected_secret);
    }

    #[test]
    fn issued_tool_lease_is_home_scoped() {
        let home_a = scoped_tool_lease_home("loongclaw-tool-lease-home-a");
        let payload = serde_json::Map::new();
        let lease = issue_tool_lease("file.read", &payload).expect("lease");

        drop(home_a);

        let _home_b = scoped_tool_lease_home("loongclaw-tool-lease-home-b");
        let validation_result = validate_tool_lease("file.read", &lease, &payload);
        let error = validation_result.expect_err("different home should reject lease");

        assert!(error.contains("signature mismatch"), "error={error}");
    }
}
