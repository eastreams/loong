use std::{collections::BTreeMap, env};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    OpenaiCompatible,
    VolcengineCustom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub kind: ProviderKind,
    pub model: String,
    #[serde(default = "default_provider_base_url")]
    pub base_url: String,
    #[serde(default = "default_openai_chat_path")]
    pub chat_completions_path: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default = "default_provider_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_provider_retry_max_attempts")]
    pub retry_max_attempts: usize,
    #[serde(default = "default_provider_retry_initial_backoff_ms")]
    pub retry_initial_backoff_ms: u64,
    #[serde(default = "default_provider_retry_max_backoff_ms")]
    pub retry_max_backoff_ms: u64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::OpenaiCompatible,
            model: "gpt-4o-mini".to_owned(),
            base_url: default_provider_base_url(),
            chat_completions_path: default_openai_chat_path(),
            endpoint: None,
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            headers: BTreeMap::new(),
            temperature: default_temperature(),
            max_tokens: None,
            request_timeout_ms: default_provider_timeout_ms(),
            retry_max_attempts: default_provider_retry_max_attempts(),
            retry_initial_backoff_ms: default_provider_retry_initial_backoff_ms(),
            retry_max_backoff_ms: default_provider_retry_max_backoff_ms(),
        }
    }
}

impl ProviderConfig {
    pub fn endpoint(&self) -> String {
        match self.kind {
            ProviderKind::OpenaiCompatible => {
                let base = self.base_url.trim_end_matches('/');
                let path = self.chat_completions_path.trim();
                if path.is_empty() {
                    format!("{base}/v1/chat/completions")
                } else if path.starts_with('/') {
                    format!("{base}{path}")
                } else {
                    format!("{base}/{path}")
                }
            }
            ProviderKind::VolcengineCustom => self.endpoint.clone().unwrap_or_else(|| {
                "https://ark.cn-beijing.volces.com/api/v3/chat/completions".to_owned()
            }),
        }
    }

    pub fn api_key(&self) -> Option<String> {
        if let Some(raw) = self.api_key.as_deref() {
            let value = raw.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
        if let Some(env_key) = self.api_key_env.as_deref() {
            let value = env::var(env_key).ok()?;
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
        None
    }
}

fn default_provider_base_url() -> String {
    "https://api.openai.com".to_owned()
}

fn default_openai_chat_path() -> String {
    "/v1/chat/completions".to_owned()
}

const fn default_temperature() -> f64 {
    0.2
}

const fn default_provider_timeout_ms() -> u64 {
    30_000
}

const fn default_provider_retry_max_attempts() -> usize {
    3
}

const fn default_provider_retry_initial_backoff_ms() -> u64 {
    300
}

const fn default_provider_retry_max_backoff_ms() -> u64 {
    3_000
}
