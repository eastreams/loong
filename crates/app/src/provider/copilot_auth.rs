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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Serializes tests that mutate the global `COPILOT_API_KEY_CACHE`.
    static CACHE_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn cached_copilot_api_key_returns_none_when_empty() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        clear_cache();
        assert_eq!(cached_copilot_api_key(), None);
    }

    #[test]
    fn cache_hit_returns_token_when_not_expired() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
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
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
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
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
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
}
