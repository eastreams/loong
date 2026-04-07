//! GitHub Copilot provider authentication.
//!
//! Uses VS Code's OAuth client ID (`Iv1.b507a08c87ecfe98`) and editor headers
//! for the Copilot token endpoint. This is the same approach used by ZeroClaw,
//! LiteLLM, Codex CLI, and other third-party integrations. The endpoint is
//! private and undocumented — GitHub could change or revoke access at any time.

use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::CliResult;

const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const TOKEN_REFRESH_BUFFER_SECS: i64 = 120;

const EDITOR_HEADERS: [(&str, &str); 3] = [
    ("Editor-Version", "vscode/1.85.1"),
    ("Editor-Plugin-Version", "copilot/1.155.0"),
    ("User-Agent", "GithubCopilot/1.155.0"),
];

const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

static COPILOT_API_KEY_CACHE: LazyLock<Mutex<Option<CachedApiKey>>> =
    LazyLock::new(|| Mutex::new(None));

struct CachedApiKey {
    token: String,
    expires_at: i64,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Returns a cached Copilot API key if one exists and has not expired.
pub fn cached_copilot_api_key() -> Option<String> {
    let cache = COPILOT_API_KEY_CACHE.lock().ok()?;
    let key = cache.as_ref()?;
    if key.expires_at > now_unix() + TOKEN_REFRESH_BUFFER_SECS {
        Some(key.token.clone())
    } else {
        None
    }
}

/// Ensures a valid Copilot API key is in the static cache.
pub async fn ensure_copilot_api_key(github_token: &str) -> CliResult<()> {
    if cached_copilot_api_key().is_some() {
        return Ok(());
    }
    let api_key = exchange_for_copilot_api_key(github_token).await?;
    let mut cache = COPILOT_API_KEY_CACHE
        .lock()
        .map_err(|e| format!("copilot cache lock poisoned: {e}"))?;
    *cache = Some(api_key);
    Ok(())
}

#[derive(Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    #[allow(dead_code)]
    expires_in: u64,
}

#[derive(Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    error: Option<String>,
}

async fn exchange_for_copilot_api_key(github_token: &str) -> CliResult<CachedApiKey> {
    let client = reqwest::Client::new();
    let mut request = client
        .get(COPILOT_TOKEN_URL)
        .header("Authorization", format!("token {github_token}"))
        .header("Accept", "application/json");
    for (key, value) in &EDITOR_HEADERS {
        request = request.header(*key, *value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Copilot token exchange failed: {e}"))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        clear_cache();
        return Err(
            "GitHub token expired or Copilot subscription inactive. \
             Run `loong onboard` to re-authenticate."
                .to_owned(),
        );
    }
    if !status.is_success() {
        return Err(format!(
            "Copilot token exchange failed with status {status}"
        ));
    }
    let body: CopilotTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Copilot token response: {e}"))?;
    Ok(CachedApiKey {
        token: body.token,
        expires_at: body.expires_at,
    })
}

fn clear_cache() {
    if let Ok(mut cache) = COPILOT_API_KEY_CACHE.lock() {
        *cache = None;
    }
}

/// Runs the OAuth Device Code Flow to obtain a GitHub OAuth token.
pub async fn device_code_login() -> CliResult<String> {
    let client = reqwest::Client::new();
    let code_response: DeviceCodeResponse = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", GITHUB_CLIENT_ID), ("scope", "read:user")])
        .send()
        .await
        .map_err(|e| format!("Failed to request device code: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse device code response: {e}"))?;

    tracing::warn!(
        "Open {} and enter code: {}",
        code_response.verification_uri,
        code_response.user_code
    );
    let mut interval = std::time::Duration::from_secs(code_response.interval.max(5));
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(code_response.expires_in);

    loop {
        tokio::time::sleep(interval).await;
        if std::time::Instant::now() > deadline {
            return Err("Authorization timed out. Please try again.".to_owned());
        }
        let response: TokenPollResponse = client
            .post(GITHUB_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", GITHUB_CLIENT_ID),
                ("device_code", code_response.device_code.as_str()),
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:device_code",
                ),
            ])
            .send()
            .await
            .map_err(|e| format!("Token poll failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse token poll response: {e}"))?;

        if let Some(token) = response.access_token {
            return Ok(token);
        }
        match response.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => interval += std::time::Duration::from_secs(5),
            Some("expired_token") => {
                return Err("Authorization timed out. Please try again.".to_owned());
            }
            Some("access_denied") => {
                return Err("Authorization denied.".to_owned());
            }
            Some(other) => {
                return Err(format!("Unexpected error during authorization: {other}"));
            }
            None => continue,
        }
    }
}

#[cfg(test)]
#[allow(dead_code)] // Used by auth_profile_runtime tests (Task 4).
pub(crate) fn set_cached_key_for_test(token: &str, expires_at: i64) {
    let mut cache = COPILOT_API_KEY_CACHE.lock().unwrap();
    *cache = Some(CachedApiKey {
        token: token.to_owned(),
        expires_at,
    });
}

#[cfg(test)]
#[allow(dead_code)] // Used by auth_profile_runtime tests (Task 4).
pub(crate) fn clear_cache_for_test() {
    clear_cache();
}

#[cfg(test)]
#[allow(dead_code)] // Used by auth_profile_runtime tests (Task 4).
pub(crate) fn now_unix_for_test() -> i64 {
    now_unix()
}

/// Serializes tests that mutate the global `COPILOT_API_KEY_CACHE`.
/// Hold the returned guard for the duration of any test that reads
/// or writes the cache.
#[cfg(test)]
pub(crate) fn acquire_cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
    CACHE_TEST_LOCK.lock().unwrap()
}

#[cfg(test)]
static CACHE_TEST_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_copilot_api_key_returns_none_when_empty() {
        let _guard = acquire_cache_test_lock();
        clear_cache();
        assert_eq!(cached_copilot_api_key(), None);
    }

    #[test]
    fn cache_hit_returns_token_when_not_expired() {
        let _guard = acquire_cache_test_lock();
        clear_cache();

        let mut cache = COPILOT_API_KEY_CACHE.lock().unwrap();
        *cache = Some(CachedApiKey {
            token: "test-copilot-key".to_owned(),
            expires_at: now_unix() + 3600,
        });
        drop(cache);

        let result = cached_copilot_api_key();
        assert_eq!(result, Some("test-copilot-key".to_owned()));
        clear_cache();
    }

    #[test]
    fn cache_miss_when_token_within_refresh_buffer() {
        let _guard = acquire_cache_test_lock();
        clear_cache();

        let mut cache = COPILOT_API_KEY_CACHE.lock().unwrap();
        *cache = Some(CachedApiKey {
            token: "about-to-expire".to_owned(),
            expires_at: now_unix() + 60,
        });
        drop(cache);

        let result = cached_copilot_api_key();
        assert_eq!(result, None);
        clear_cache();
    }

    #[test]
    fn clear_cache_removes_stored_key() {
        let _guard = acquire_cache_test_lock();
        clear_cache();

        let mut cache = COPILOT_API_KEY_CACHE.lock().unwrap();
        *cache = Some(CachedApiKey {
            token: "will-be-cleared".to_owned(),
            expires_at: now_unix() + 3600,
        });
        drop(cache);

        clear_cache();
        assert_eq!(cached_copilot_api_key(), None);
    }

    #[test]
    fn copilot_token_response_deserializes() {
        let json = r#"{"token":"tid=abc;exp=123","expires_at":1700000000}"#;
        let parsed: CopilotTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.token, "tid=abc;exp=123");
        assert_eq!(parsed.expires_at, 1700000000);
    }

    #[test]
    fn device_code_response_deserializes() {
        let json = r#"{
            "device_code": "abc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
            "interval": 5,
            "expires_in": 900
        }"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.user_code, "ABCD-1234");
        assert_eq!(parsed.interval, 5);
    }

    #[test]
    fn token_poll_response_deserializes_success() {
        let json = r#"{"access_token":"ghu_xxxx","token_type":"bearer","scope":"read:user"}"#;
        let parsed: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token.as_deref(), Some("ghu_xxxx"));
        assert_eq!(parsed.error, None);
    }

    #[test]
    fn token_poll_response_deserializes_pending() {
        let json = r#"{"error":"authorization_pending"}"#;
        let parsed: TokenPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token, None);
        assert_eq!(parsed.error.as_deref(), Some("authorization_pending"));
    }
}
