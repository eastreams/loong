use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
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
            let existing_secret = read_tool_lease_secret_file(secret_path)?;
            let Some(existing_secret) = existing_secret else {
                let message = format!(
                    "tool_lease_authority_unavailable: secret file appeared without readable contents at {}",
                    secret_path.display()
                );
                return Err(message);
            };
            Ok(existing_secret)
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
    let mut options = OpenOptions::new();
    options.write(true);
    options.create_new(true);

    let file = options.open(secret_path);
    let mut file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(CreateToolLeaseSecretError::AlreadyExists);
        }
        Err(error) => return Err(CreateToolLeaseSecretError::Io(error)),
    };

    let write_result = writeln!(file, "{secret}");
    if let Err(error) = write_result {
        let _ = fs::remove_file(secret_path);
        return Err(CreateToolLeaseSecretError::Io(error));
    }

    let sync_result = file.sync_all();
    if let Err(error) = sync_result {
        let _ = fs::remove_file(secret_path);
        return Err(CreateToolLeaseSecretError::Io(error));
    }

    Ok(())
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
pub(super) fn clear_tool_lease_secret_cache_for_tests() {
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
    use tempfile::TempDir;

    use super::clear_tool_lease_secret_cache_for_tests;
    use super::default_tool_lease_secret_path;
    use super::issue_tool_lease;
    use super::read_tool_lease_secret_file;
    use super::validate_tool_lease;
    use crate::test_support::ScopedEnv;

    fn scoped_tool_lease_home() -> (TempDir, ScopedEnv) {
        let temp_home = TempDir::new().expect("temp home");
        let mut env = ScopedEnv::new();
        env.set("LOONG_HOME", temp_home.path());
        clear_tool_lease_secret_cache_for_tests();
        (temp_home, env)
    }

    #[test]
    fn issue_tool_lease_persists_secret_under_loong_home() {
        let (_temp_home, _env) = scoped_tool_lease_home();
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
        let (_temp_home, _env) = scoped_tool_lease_home();
        let payload = serde_json::Map::new();

        let lease = issue_tool_lease("file.read", &payload).expect("lease");

        clear_tool_lease_secret_cache_for_tests();

        let validation_result = validate_tool_lease("file.read", &lease, &payload);

        validation_result.expect("lease should survive authority reload");
    }

    #[test]
    fn issued_tool_lease_is_home_scoped() {
        let home_a = TempDir::new().expect("temp home");
        let mut env = ScopedEnv::new();
        env.set("LOONG_HOME", home_a.path());
        clear_tool_lease_secret_cache_for_tests();
        let payload = serde_json::Map::new();
        let lease = issue_tool_lease("file.read", &payload).expect("lease");

        let temp_home_b = TempDir::new().expect("temp home");
        env.set("LOONG_HOME", temp_home_b.path());
        clear_tool_lease_secret_cache_for_tests();

        let validation_result = validate_tool_lease("file.read", &lease, &payload);
        let error = validation_result.expect_err("different home should reject lease");

        assert!(error.contains("signature mismatch"), "error={error}");
    }
}
