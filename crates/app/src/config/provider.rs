use std::{collections::BTreeMap, env};

use serde::{Deserialize, Serialize};

use super::shared::{validate_env_pointer_field, ConfigValidationIssue, EnvPointerValidationHint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub base_url: &'static str,
    pub chat_completions_path: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ReasoningEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::None => "none",
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(alias = "anthropic_compatible")]
    Anthropic,
    #[serde(alias = "kimi_compatible")]
    Kimi,
    #[serde(alias = "kimi_coding_compatible")]
    KimiCoding,
    #[serde(alias = "minimax_compatible")]
    Minimax,
    #[serde(alias = "ollama_compatible")]
    Ollama,
    #[default]
    #[serde(alias = "openai_compatible")]
    Openai,
    #[serde(alias = "openrouter_compatible")]
    Openrouter,
    #[serde(alias = "volcengine_custom", alias = "volcengine_compatible")]
    Volcengine,
    #[serde(alias = "xai_compatible")]
    Xai,
    #[serde(alias = "zai_compatible")]
    Zai,
    #[serde(alias = "zhipu_compatible")]
    Zhipu,
    #[serde(alias = "deepseek_compatible")]
    Deepseek,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub kind: ProviderKind,
    #[serde(default = "default_provider_model")]
    pub model: String,
    #[serde(default = "default_provider_base_url")]
    pub base_url: String,
    #[serde(default = "default_openai_chat_path")]
    pub chat_completions_path: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub oauth_access_token: Option<String>,
    #[serde(default)]
    pub oauth_access_token_env: Option<String>,
    #[serde(default)]
    pub preferred_models: Vec<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
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
            kind: ProviderKind::Openai,
            model: default_provider_model(),
            base_url: default_provider_base_url(),
            chat_completions_path: default_openai_chat_path(),
            endpoint: None,
            models_endpoint: None,
            api_key: None,
            api_key_env: Some(default_provider_api_key_env().to_owned()),
            oauth_access_token: None,
            oauth_access_token_env: None,
            preferred_models: Vec::new(),
            reasoning_effort: None,
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
    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        let api_key_example = self
            .kind
            .default_api_key_env()
            .unwrap_or("PROVIDER_API_KEY");
        if let Err(issue) = validate_env_pointer_field(
            "provider.api_key_env",
            self.api_key_env.as_deref(),
            EnvPointerValidationHint {
                inline_field_path: "provider.api_key",
                example_env_name: api_key_example,
                detect_telegram_token_shape: false,
            },
        ) {
            issues.push(issue);
        }
        let oauth_example = self
            .kind
            .default_oauth_access_token_env()
            .unwrap_or("PROVIDER_OAUTH_ACCESS_TOKEN");
        if let Err(issue) = validate_env_pointer_field(
            "provider.oauth_access_token_env",
            self.oauth_access_token_env.as_deref(),
            EnvPointerValidationHint {
                inline_field_path: "provider.oauth_access_token",
                example_env_name: oauth_example,
                detect_telegram_token_shape: false,
            },
        ) {
            issues.push(issue);
        }
        issues
    }

    pub fn endpoint(&self) -> String {
        if let Some(endpoint) = non_empty(self.endpoint.as_deref()) {
            return endpoint.to_owned();
        }

        let profile = self.kind.profile();
        let resolved_base_url =
            self.resolve_base_url(profile.base_url, default_provider_base_url().as_str());
        let resolved_chat_path = self.resolve_chat_path(
            profile.chat_completions_path,
            default_openai_chat_path().as_str(),
            default_provider_base_url().as_str(),
        );
        join_base_with_path(
            &resolved_base_url,
            &resolved_chat_path,
            default_openai_chat_path().as_str(),
        )
    }

    pub fn models_endpoint(&self) -> String {
        if let Some(endpoint) = non_empty(self.models_endpoint.as_deref()) {
            return endpoint.to_owned();
        }

        let profile = self.kind.profile();
        let resolved_base_url =
            self.resolve_base_url(profile.base_url, default_provider_base_url().as_str());
        let resolved_chat_path = self.resolve_chat_path(
            profile.chat_completions_path,
            default_openai_chat_path().as_str(),
            default_provider_base_url().as_str(),
        );
        let models_path = derive_models_path(&resolved_chat_path);
        join_base_with_path(&resolved_base_url, &models_path, "/v1/models")
    }

    #[cfg(test)]
    pub fn default_api_key_env(&self) -> Option<String> {
        self.kind.default_api_key_env().map(str::to_owned)
    }

    #[cfg(test)]
    pub fn default_oauth_access_token_env(&self) -> Option<String> {
        self.kind
            .default_oauth_access_token_env()
            .map(str::to_owned)
    }

    pub fn authorization_header(&self) -> Option<String> {
        if let Some(token) = self.oauth_access_token() {
            return Some(format!("Bearer {token}"));
        }
        self.api_key().map(|key| format!("Bearer {key}"))
    }

    pub fn resolved_model(&self) -> Option<String> {
        let trimmed = self.model.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            return Some(trimmed.to_owned());
        }
        self.kind.default_model().map(str::to_owned)
    }

    pub fn model_selection_requires_fetch(&self) -> bool {
        self.resolved_model().is_none()
    }

    pub fn oauth_access_token(&self) -> Option<String> {
        if let Some(raw) = self.oauth_access_token.as_deref() {
            let value = raw.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }

        let mut env_keys = Vec::new();
        push_unique_env_key(&mut env_keys, self.oauth_access_token_env.as_deref());
        push_unique_env_key(&mut env_keys, self.kind.default_oauth_access_token_env());
        for alias in self.kind.oauth_access_token_env_aliases() {
            push_unique_env_key(&mut env_keys, Some(alias));
        }

        first_non_empty_env_value(&env_keys)
    }

    fn resolve_base_url(&self, profile_default: &str, openai_default: &str) -> String {
        let base = self.base_url.trim();
        if base.is_empty() {
            return profile_default.to_owned();
        }
        if self.kind != ProviderKind::Openai
            && is_same_base_url(base, openai_default)
            && (self.chat_completions_path.trim().is_empty()
                || is_same_chat_path(
                    self.chat_completions_path.as_str(),
                    default_openai_chat_path().as_str(),
                ))
        {
            return profile_default.to_owned();
        }
        base.to_owned()
    }

    fn resolve_chat_path(
        &self,
        profile_default: &str,
        openai_default_path: &str,
        openai_default_base: &str,
    ) -> String {
        let path = self.chat_completions_path.trim();
        if path.is_empty() {
            return profile_default.to_owned();
        }
        if self.kind != ProviderKind::Openai
            && is_same_chat_path(path, openai_default_path)
            && (self.base_url.trim().is_empty()
                || is_same_base_url(self.base_url.as_str(), openai_default_base))
        {
            return profile_default.to_owned();
        }
        normalize_api_path(path)
    }

    pub fn api_key(&self) -> Option<String> {
        if let Some(raw) = self.api_key.as_deref() {
            let value = raw.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }

        let mut env_keys = Vec::new();
        push_unique_env_key(&mut env_keys, self.api_key_env.as_deref());
        push_unique_env_key(&mut env_keys, self.kind.default_api_key_env());
        for alias in self.kind.api_key_env_aliases() {
            push_unique_env_key(&mut env_keys, Some(alias));
        }

        first_non_empty_env_value(&env_keys)
    }

    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl ProviderKind {
    #[cfg(test)]
    pub const fn all_sorted() -> &'static [ProviderKind] {
        &[
            ProviderKind::Anthropic,
            ProviderKind::Deepseek,
            ProviderKind::Kimi,
            ProviderKind::KimiCoding,
            ProviderKind::Minimax,
            ProviderKind::Ollama,
            ProviderKind::Openai,
            ProviderKind::Openrouter,
            ProviderKind::Volcengine,
            ProviderKind::Xai,
            ProviderKind::Zai,
            ProviderKind::Zhipu,
        ]
    }

    #[cfg(test)]
    pub const fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Deepseek => "deepseek",
            ProviderKind::Kimi => "kimi",
            ProviderKind::KimiCoding => "kimi_coding",
            ProviderKind::Minimax => "minimax",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Openai => "openai",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Volcengine => "volcengine",
            ProviderKind::Xai => "xai",
            ProviderKind::Zai => "zai",
            ProviderKind::Zhipu => "zhipu",
        }
    }

    pub const fn profile(self) -> ProviderProfile {
        match self {
            ProviderKind::Anthropic => ProviderProfile {
                id: "anthropic",
                base_url: "https://api.anthropic.com/v1",
                chat_completions_path: "/chat/completions",
            },
            ProviderKind::Deepseek => ProviderProfile {
                id: "deepseek",
                base_url: "https://api.deepseek.com",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::Kimi => ProviderProfile {
                id: "kimi",
                base_url: "https://api.moonshot.cn",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::KimiCoding => ProviderProfile {
                id: "kimi_coding",
                base_url: "https://api.kimi.com",
                chat_completions_path: "/coding/v1/chat/completions",
            },
            ProviderKind::Minimax => ProviderProfile {
                id: "minimax",
                base_url: "https://api.minimaxi.com",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::Ollama => ProviderProfile {
                id: "ollama",
                base_url: "http://127.0.0.1:11434",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::Openai => ProviderProfile {
                id: "openai",
                base_url: "https://api.openai.com",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::Openrouter => ProviderProfile {
                id: "openrouter",
                base_url: "https://openrouter.ai",
                chat_completions_path: "/api/v1/chat/completions",
            },
            ProviderKind::Volcengine => ProviderProfile {
                id: "volcengine",
                base_url: "https://ark.cn-beijing.volces.com",
                chat_completions_path: "/api/v3/chat/completions",
            },
            ProviderKind::Xai => ProviderProfile {
                id: "xai",
                base_url: "https://api.x.ai",
                chat_completions_path: "/v1/chat/completions",
            },
            ProviderKind::Zai => ProviderProfile {
                id: "zai",
                base_url: "https://api.z.ai",
                chat_completions_path: "/api/paas/v4/chat/completions",
            },
            ProviderKind::Zhipu => ProviderProfile {
                id: "zhipu",
                base_url: "https://open.bigmodel.cn",
                chat_completions_path: "/api/paas/v4/chat/completions",
            },
        }
    }

    pub const fn default_api_key_env(self) -> Option<&'static str> {
        match self {
            ProviderKind::Anthropic => Some("ANTHROPIC_API_KEY"),
            ProviderKind::Deepseek => Some("DEEPSEEK_API_KEY"),
            ProviderKind::Kimi => Some("MOONSHOT_API_KEY"),
            ProviderKind::KimiCoding => Some("KIMI_API_KEY"),
            ProviderKind::Minimax => Some("MINIMAX_API_KEY"),
            ProviderKind::Ollama => None,
            ProviderKind::Openai => Some("OPENAI_API_KEY"),
            ProviderKind::Openrouter => Some("OPENROUTER_API_KEY"),
            ProviderKind::Volcengine => Some("ARK_API_KEY"),
            ProviderKind::Xai => Some("XAI_API_KEY"),
            ProviderKind::Zai => Some("ZAI_API_KEY"),
            ProviderKind::Zhipu => Some("ZHIPUAI_API_KEY"),
        }
    }

    pub const fn api_key_env_aliases(self) -> &'static [&'static str] {
        match self {
            ProviderKind::Kimi => &["KIMI_API_KEY"],
            ProviderKind::KimiCoding => &["KIMICODE_API_KEY"],
            ProviderKind::Zhipu => &["ZHIPU_API_KEY"],
            _ => &[],
        }
    }

    pub const fn default_model(self) -> Option<&'static str> {
        match self {
            ProviderKind::KimiCoding => Some("kimi-for-coding"),
            _ => None,
        }
    }

    pub const fn default_user_agent(self) -> Option<&'static str> {
        match self {
            ProviderKind::KimiCoding => Some("KimiCLI/LoongClaw"),
            _ => None,
        }
    }

    pub const fn default_oauth_access_token_env(self) -> Option<&'static str> {
        match self {
            ProviderKind::Openai => Some("OPENAI_CODEX_OAUTH_TOKEN"),
            ProviderKind::Volcengine => Some("VOLCENGINE_CODING_PLAN_OAUTH_TOKEN"),
            _ => None,
        }
    }

    pub const fn oauth_access_token_env_aliases(self) -> &'static [&'static str] {
        match self {
            ProviderKind::Openai => &["OPENAI_OAUTH_ACCESS_TOKEN"],
            ProviderKind::Volcengine => &["ARK_OAUTH_ACCESS_TOKEN"],
            _ => &[],
        }
    }
}

fn default_provider_model() -> String {
    "auto".to_owned()
}

fn default_provider_base_url() -> String {
    "https://api.openai.com".to_owned()
}

const fn default_provider_api_key_env() -> &'static str {
    "OPENAI_API_KEY"
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

fn first_non_empty_env_value(keys: &[String]) -> Option<String> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn push_unique_env_key(keys: &mut Vec<String>, maybe_key: Option<&str>) {
    let Some(raw) = maybe_key else {
        return;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    if keys.iter().any(|existing| existing == trimmed) {
        return;
    }
    keys.push(trimmed.to_owned());
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    let raw = value?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

fn normalize_api_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('/') {
        return trimmed.to_owned();
    }
    format!("/{trimmed}")
}

fn is_same_base_url(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

fn is_same_chat_path(left: &str, right: &str) -> bool {
    normalize_api_path(left) == normalize_api_path(right)
}

fn join_base_with_path(base_url: &str, path: &str, fallback_path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let path = normalize_api_path(path);
    if path.is_empty() {
        return format!("{base}{}", normalize_api_path(fallback_path));
    }
    format!("{base}{path}")
}

fn derive_models_path(chat_path: &str) -> String {
    let normalized = normalize_api_path(chat_path);
    if normalized.is_empty() {
        return "/v1/models".to_owned();
    }

    if let Some(prefix) = normalized.strip_suffix("/chat/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/models");
    }
    if let Some(prefix) = normalized.strip_suffix("/completions") {
        let prefix = if prefix.is_empty() { "" } else { prefix };
        return format!("{prefix}/models");
    }

    "/v1/models".to_owned()
}
