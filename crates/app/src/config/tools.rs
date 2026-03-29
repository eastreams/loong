use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use super::shared::{ConfigValidationIssue, expand_path, validate_numeric_range};

pub const DEFAULT_WEB_FETCH_MAX_BYTES: usize = 1024 * 1024;
pub const DEFAULT_WEB_FETCH_TIMEOUT_SECONDS: u64 = 15;
pub const DEFAULT_WEB_FETCH_MAX_REDIRECTS: usize = 3;
pub const DEFAULT_BROWSER_MAX_SESSIONS: usize = 8;
pub const DEFAULT_BROWSER_MAX_LINKS: usize = 40;
pub const DEFAULT_BROWSER_MAX_TEXT_CHARS: usize = 6000;
pub const DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS: usize = 20_000;
pub const DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS: usize = 150_000;
pub(crate) const MIN_WEB_FETCH_MAX_BYTES: usize = 1024;
pub const MAX_WEB_FETCH_MAX_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MIN_WEB_FETCH_TIMEOUT_SECONDS: usize = 1;
pub(crate) const MAX_WEB_FETCH_TIMEOUT_SECONDS: usize = 120;
pub(crate) const MAX_WEB_FETCH_MAX_REDIRECTS: usize = 10;
pub(crate) const MIN_BROWSER_MAX_SESSIONS: usize = 1;
pub const MAX_BROWSER_MAX_SESSIONS: usize = 32;
pub(crate) const MIN_BROWSER_MAX_LINKS: usize = 1;
pub const MAX_BROWSER_MAX_LINKS: usize = 200;
pub(crate) const MIN_BROWSER_MAX_TEXT_CHARS: usize = 256;
pub const MAX_BROWSER_MAX_TEXT_CHARS: usize = 20_000;
pub(crate) const MIN_RUNTIME_SELF_MAX_SOURCE_CHARS: usize = 256;
pub const MAX_RUNTIME_SELF_MAX_SOURCE_CHARS: usize = 100_000;
pub(crate) const MIN_RUNTIME_SELF_MAX_TOTAL_CHARS: usize = 1_024;
pub const MAX_RUNTIME_SELF_MAX_TOTAL_CHARS: usize = 500_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolConfig {
    #[serde(default)]
    pub file_root: Option<String>,
    /// Commands to allow. Defaults to empty — no commands are allowed unless
    /// explicitly configured.
    #[serde(default = "default_shell_allow")]
    pub shell_allow: Vec<String>,
    /// Commands to hard-deny.
    #[serde(default)]
    pub shell_deny: Vec<String>,
    /// Default policy for unknown commands: "deny" (default) or "allow".
    #[serde(default = "default_shell_default_mode")]
    pub shell_default_mode: String,
    #[serde(default)]
    pub consent: ToolConsentConfig,
    #[serde(default)]
    pub approval: GovernedToolApprovalConfig,
    #[serde(default)]
    pub sessions: SessionToolConfig,
    #[serde(default)]
    pub messages: MessageToolConfig,
    #[serde(default)]
    pub delegate: DelegateToolConfig,
    #[serde(default)]
    pub runtime_self: RuntimeSelfToolConfig,
    #[serde(default)]
    pub browser: BrowserToolConfig,
    #[serde(default)]
    pub browser_companion: BrowserCompanionToolConfig,
    #[serde(default)]
    pub web: WebToolConfig,
    #[serde(default)]
    pub web_search: WebSearchToolConfig,
    #[serde(default)]
    pub tool_execution: ToolExecutionToolConfig,
    #[serde(default)]
    pub autonomy_profile: AutonomyProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolExecutionToolConfig {
    #[serde(default)]
    pub default_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub per_tool_timeout: BTreeMap<String, u64>,
}

const AUTONOMY_PROFILE_IDS: [&str; 3] =
    ["discovery_only", "guided_acquisition", "bounded_autonomous"];

pub const AUTONOMY_PROFILE_VALID_VALUES: &str =
    "discovery_only, guided_acquisition, bounded_autonomous";

#[repr(usize)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyProfile {
    #[default]
    DiscoveryOnly,
    GuidedAcquisition,
    BoundedAutonomous,
}

impl AutonomyProfile {
    const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::DiscoveryOnly),
            1 => Some(Self::GuidedAcquisition),
            2 => Some(Self::BoundedAutonomous),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiscoveryOnly => AUTONOMY_PROFILE_IDS[0],
            Self::GuidedAcquisition => AUTONOMY_PROFILE_IDS[1],
            Self::BoundedAutonomous => AUTONOMY_PROFILE_IDS[2],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolConsentMode {
    Prompt,
    Auto,
    #[default]
    Full,
}

impl ToolConsentMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolConsentConfig {
    #[serde(default)]
    pub default_mode: ToolConsentMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GovernedToolApprovalMode {
    #[default]
    Disabled,
    MediumBalanced,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GovernedToolApprovalConfig {
    #[serde(default)]
    pub mode: GovernedToolApprovalMode,
    #[serde(default)]
    pub approved_calls: Vec<String>,
    #[serde(default)]
    pub denied_calls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SessionVisibility {
    #[serde(rename = "self")]
    SelfOnly,
    #[default]
    #[serde(rename = "children")]
    Children,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionToolConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub visibility: SessionVisibility,
    #[serde(default = "default_session_list_limit")]
    pub list_limit: usize,
    #[serde(default = "default_session_history_limit")]
    pub history_limit: usize,
    #[serde(default)]
    pub allow_mutation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MessageToolConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegateToolConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_delegate_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_delegate_max_active_children")]
    pub max_active_children: usize,
    #[serde(default = "default_delegate_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_delegate_child_tool_allowlist")]
    pub child_tool_allowlist: Vec<String>,
    #[serde(default)]
    pub allow_shell_in_child: bool,
    #[serde(default)]
    pub child_runtime: DelegateChildRuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DelegateChildRuntimeConfig {
    #[serde(default)]
    pub web: DelegateChildWebRuntimeConfig,
    #[serde(default)]
    pub browser: DelegateChildBrowserRuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DelegateChildWebRuntimeConfig {
    #[serde(default)]
    pub allow_private_hosts: Option<bool>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub max_redirects: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DelegateChildBrowserRuntimeConfig {
    #[serde(default)]
    pub max_sessions: Option<usize>,
    #[serde(default)]
    pub max_links: Option<usize>,
    #[serde(default)]
    pub max_text_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserToolConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_browser_max_sessions")]
    pub max_sessions: usize,
    #[serde(default = "default_browser_max_links")]
    pub max_links: usize,
    #[serde(default = "default_browser_max_text_chars")]
    pub max_text_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserCompanionToolConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default = "default_browser_companion_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allow_private_hosts: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSelfToolConfig {
    #[serde(default = "default_runtime_self_max_source_chars")]
    pub max_source_chars: usize,
    #[serde(default = "default_runtime_self_max_total_chars")]
    pub max_total_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebToolConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allow_private_hosts: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default = "default_web_fetch_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_web_fetch_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_web_fetch_max_redirects")]
    pub max_redirects: usize,
}

pub const DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_WEB_SEARCH_MAX_RESULTS: usize = 5;
pub const WEB_SEARCH_PROVIDER_DUCKDUCKGO: &str = "duckduckgo";
pub const WEB_SEARCH_PROVIDER_BRAVE: &str = "brave";
pub const WEB_SEARCH_PROVIDER_TAVILY: &str = "tavily";
pub const WEB_SEARCH_PROVIDER_PERPLEXITY: &str = "perplexity";
pub const WEB_SEARCH_PROVIDER_EXA: &str = "exa";
pub const WEB_SEARCH_PROVIDER_JINA: &str = "jina";
pub const DEFAULT_WEB_SEARCH_PROVIDER: &str = WEB_SEARCH_PROVIDER_DUCKDUCKGO;
#[cfg(feature = "tool-websearch")]
pub(crate) const WEB_SEARCH_PROVIDER_SCHEMA_VALUES: &[&str] = &[
    WEB_SEARCH_PROVIDER_DUCKDUCKGO,
    "ddg",
    WEB_SEARCH_PROVIDER_BRAVE,
    WEB_SEARCH_PROVIDER_TAVILY,
    WEB_SEARCH_PROVIDER_PERPLEXITY,
    "perplexity_search",
    WEB_SEARCH_PROVIDER_EXA,
    WEB_SEARCH_PROVIDER_JINA,
    "jinaai",
    "jina-ai",
];
pub const WEB_SEARCH_PROVIDER_VALID_VALUES: &str = "duckduckgo (or ddg), brave, tavily, perplexity (or perplexity_search), exa, jina (or jinaai / jina-ai)";
pub const WEB_SEARCH_BRAVE_API_KEY_ENV: &str = "BRAVE_API_KEY";
pub const WEB_SEARCH_TAVILY_API_KEY_ENV: &str = "TAVILY_API_KEY";
pub const WEB_SEARCH_PERPLEXITY_API_KEY_ENV: &str = "PERPLEXITY_API_KEY";
pub const WEB_SEARCH_EXA_API_KEY_ENV: &str = "EXA_API_KEY";
pub const WEB_SEARCH_JINA_API_KEY_ENV: &str = "JINA_API_KEY";
pub const WEB_SEARCH_JINA_AUTH_TOKEN_ENV: &str = "JINA_AUTH_TOKEN";
pub(crate) const MIN_WEB_SEARCH_TIMEOUT_SECONDS: usize = 1;
pub(crate) const MAX_WEB_SEARCH_TIMEOUT_SECONDS: usize = 60;
pub(crate) const MIN_WEB_SEARCH_MAX_RESULTS: usize = 1;
pub(crate) const MAX_WEB_SEARCH_MAX_RESULTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebSearchProviderDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub requires_api_key: bool,
    pub default_api_key_env: Option<&'static str>,
    pub api_key_env_names: &'static [&'static str],
}

const WEB_SEARCH_EMPTY_API_KEY_ENV_NAMES: &[&str] = &[];
const WEB_SEARCH_BRAVE_API_KEY_ENV_NAMES: &[&str] = &[WEB_SEARCH_BRAVE_API_KEY_ENV];
const WEB_SEARCH_TAVILY_API_KEY_ENV_NAMES: &[&str] = &[WEB_SEARCH_TAVILY_API_KEY_ENV];
const WEB_SEARCH_PERPLEXITY_API_KEY_ENV_NAMES: &[&str] = &[WEB_SEARCH_PERPLEXITY_API_KEY_ENV];
const WEB_SEARCH_EXA_API_KEY_ENV_NAMES: &[&str] = &[WEB_SEARCH_EXA_API_KEY_ENV];
const WEB_SEARCH_JINA_API_KEY_ENV_NAMES: &[&str] =
    &[WEB_SEARCH_JINA_API_KEY_ENV, WEB_SEARCH_JINA_AUTH_TOKEN_ENV];

const WEB_SEARCH_PROVIDER_DESCRIPTORS: &[WebSearchProviderDescriptor] = &[
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_DUCKDUCKGO,
        display_name: "DuckDuckGo",
        description: "key-free HTML search fallback",
        requires_api_key: false,
        default_api_key_env: None,
        api_key_env_names: WEB_SEARCH_EMPTY_API_KEY_ENV_NAMES,
    },
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_BRAVE,
        display_name: "Brave Search",
        description: "fast web API with structured results",
        requires_api_key: true,
        default_api_key_env: Some(WEB_SEARCH_BRAVE_API_KEY_ENV),
        api_key_env_names: WEB_SEARCH_BRAVE_API_KEY_ENV_NAMES,
    },
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_TAVILY,
        display_name: "Tavily",
        description: "search API that works well as a grounded research backend",
        requires_api_key: true,
        default_api_key_env: Some(WEB_SEARCH_TAVILY_API_KEY_ENV),
        api_key_env_names: WEB_SEARCH_TAVILY_API_KEY_ENV_NAMES,
    },
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_PERPLEXITY,
        display_name: "Perplexity Search",
        description: "grounded search API with returned snippets and citations",
        requires_api_key: true,
        default_api_key_env: Some(WEB_SEARCH_PERPLEXITY_API_KEY_ENV),
        api_key_env_names: WEB_SEARCH_PERPLEXITY_API_KEY_ENV_NAMES,
    },
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_EXA,
        display_name: "Exa",
        description: "semantic search API with highlights and result text",
        requires_api_key: true,
        default_api_key_env: Some(WEB_SEARCH_EXA_API_KEY_ENV),
        api_key_env_names: WEB_SEARCH_EXA_API_KEY_ENV_NAMES,
    },
    WebSearchProviderDescriptor {
        id: WEB_SEARCH_PROVIDER_JINA,
        display_name: "Jina Search",
        description: "grounded search digest via s.jina.ai",
        requires_api_key: true,
        default_api_key_env: Some(WEB_SEARCH_JINA_API_KEY_ENV),
        api_key_env_names: WEB_SEARCH_JINA_API_KEY_ENV_NAMES,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchToolConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_web_search_provider")]
    pub default_provider: String,
    #[serde(default = "default_web_search_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_web_search_max_results")]
    pub max_results: usize,
    #[serde(default)]
    pub brave_api_key: Option<String>,
    #[serde(default)]
    pub tavily_api_key: Option<String>,
    #[serde(default)]
    pub perplexity_api_key: Option<String>,
    #[serde(default)]
    pub exa_api_key: Option<String>,
    #[serde(default)]
    pub jina_api_key: Option<String>,
}

fn default_shell_default_mode() -> String {
    "deny".to_owned()
}

const fn default_browser_companion_timeout_seconds() -> u64 {
    DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS
}

const fn default_runtime_self_max_source_chars() -> usize {
    DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS
}

const fn default_runtime_self_max_total_chars() -> usize {
    DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS
}

/// Default allow list used when the config file omits `shell_allow`.
///
/// Empty by design: no commands are allowed unless the user explicitly
/// configures `shell_allow` in their config file. This upholds the
/// default-deny principle — silent implicit permissions are not injected.
///
/// Also used by `ToolRuntimeConfig::default()` so the runtime fallback
/// and a freshly-parsed config file agree on the initial allow set.
pub const DEFAULT_SHELL_ALLOW: &[&str] = &[];

/// Serde default for `ToolConfig::shell_allow`.
///
/// Returns an empty list — no commands are implicitly allowed.
fn default_shell_allow() -> Vec<String> {
    DEFAULT_SHELL_ALLOW
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSkillsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_require_download_approval")]
    pub require_download_approval: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub install_root: Option<String>,
    #[serde(default = "default_auto_expose_installed")]
    pub auto_expose_installed: bool,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            shell_allow: default_shell_allow(),
            shell_deny: Vec::new(),
            shell_default_mode: default_shell_default_mode(),
            consent: ToolConsentConfig::default(),
            approval: GovernedToolApprovalConfig::default(),
            sessions: SessionToolConfig::default(),
            messages: MessageToolConfig::default(),
            delegate: DelegateToolConfig::default(),
            runtime_self: RuntimeSelfToolConfig::default(),
            browser: BrowserToolConfig::default(),
            browser_companion: BrowserCompanionToolConfig::default(),
            web: WebToolConfig::default(),
            web_search: WebSearchToolConfig::default(),
            tool_execution: ToolExecutionToolConfig::default(),
            autonomy_profile: AutonomyProfile::default(),
        }
    }
}

impl Default for BrowserCompanionToolConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: None,
            expected_version: None,
            timeout_seconds: default_browser_companion_timeout_seconds(),
            allow_private_hosts: false,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
        }
    }
}

impl Default for RuntimeSelfToolConfig {
    fn default() -> Self {
        Self {
            max_source_chars: default_runtime_self_max_source_chars(),
            max_total_chars: default_runtime_self_max_total_chars(),
        }
    }
}

impl Default for GovernedToolApprovalConfig {
    fn default() -> Self {
        Self {
            mode: GovernedToolApprovalMode::Disabled,
            approved_calls: Vec::new(),
            denied_calls: Vec::new(),
        }
    }
}

impl Default for SessionToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            visibility: SessionVisibility::default(),
            list_limit: default_session_list_limit(),
            history_limit: default_session_history_limit(),
            allow_mutation: false,
        }
    }
}

impl Default for DelegateToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_depth: default_delegate_max_depth(),
            max_active_children: default_delegate_max_active_children(),
            timeout_seconds: default_delegate_timeout_seconds(),
            child_tool_allowlist: default_delegate_child_tool_allowlist(),
            allow_shell_in_child: false,
            child_runtime: DelegateChildRuntimeConfig::default(),
        }
    }
}

impl Default for WebToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            allow_private_hosts: false,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            max_bytes: default_web_fetch_max_bytes(),
            timeout_seconds: default_web_fetch_timeout_seconds(),
            max_redirects: default_web_fetch_max_redirects(),
        }
    }
}

impl Default for BrowserToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_sessions: default_browser_max_sessions(),
            max_links: default_browser_max_links(),
            max_text_chars: default_browser_max_text_chars(),
        }
    }
}

impl Default for ExternalSkillsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            require_download_approval: default_require_download_approval(),
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            install_root: None,
            auto_expose_installed: default_auto_expose_installed(),
        }
    }
}

impl Default for WebSearchToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            default_provider: default_web_search_provider(),
            timeout_seconds: default_web_search_timeout_seconds(),
            max_results: default_web_search_max_results(),
            brave_api_key: None,
            tavily_api_key: None,
            perplexity_api_key: None,
            exa_api_key: None,
            jina_api_key: None,
        }
    }
}

impl ToolConfig {
    pub fn explicit_file_root(&self) -> Option<&str> {
        self.file_root
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn resolved_file_root(&self) -> PathBuf {
        if let Some(path) = self.explicit_file_root() {
            return expand_path(path);
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        if let Err(issue) = validate_numeric_range(
            "tools.runtime_self.max_source_chars",
            self.runtime_self.max_source_chars,
            MIN_RUNTIME_SELF_MAX_SOURCE_CHARS,
            MAX_RUNTIME_SELF_MAX_SOURCE_CHARS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.runtime_self.max_total_chars",
            self.runtime_self.max_total_chars,
            MIN_RUNTIME_SELF_MAX_TOTAL_CHARS,
            MAX_RUNTIME_SELF_MAX_TOTAL_CHARS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.browser.max_sessions",
            self.browser.max_sessions,
            MIN_BROWSER_MAX_SESSIONS,
            MAX_BROWSER_MAX_SESSIONS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.browser.max_links",
            self.browser.max_links,
            MIN_BROWSER_MAX_LINKS,
            MAX_BROWSER_MAX_LINKS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.browser.max_text_chars",
            self.browser.max_text_chars,
            MIN_BROWSER_MAX_TEXT_CHARS,
            MAX_BROWSER_MAX_TEXT_CHARS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.web.max_bytes",
            self.web.max_bytes,
            MIN_WEB_FETCH_MAX_BYTES,
            MAX_WEB_FETCH_MAX_BYTES,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.web.timeout_seconds",
            self.web.timeout_seconds as usize,
            MIN_WEB_FETCH_TIMEOUT_SECONDS,
            MAX_WEB_FETCH_TIMEOUT_SECONDS,
        ) {
            issues.push(*issue);
        }
        if let Err(issue) = validate_numeric_range(
            "tools.web.max_redirects",
            self.web.max_redirects,
            0,
            MAX_WEB_FETCH_MAX_REDIRECTS,
        ) {
            issues.push(*issue);
        }
        if self.tool_execution.default_timeout_seconds == Some(0)
            && let Err(issue) = validate_numeric_range(
                "tools.tool_execution.default_timeout_seconds",
                0,
                1,
                usize::MAX,
            )
        {
            issues.push(*issue);
        }
        for (tool_name, timeout_seconds) in &self.tool_execution.per_tool_timeout {
            if *timeout_seconds != 0 {
                continue;
            }
            let field_path = format!("tools.tool_execution.per_tool_timeout.{tool_name}");
            if let Err(issue) = validate_numeric_range(&field_path, 0, 1, usize::MAX) {
                issues.push(*issue);
            }
        }
        if let Some(max_sessions) = self.delegate.child_runtime.browser.max_sessions
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.browser.max_sessions",
                max_sessions,
                MIN_BROWSER_MAX_SESSIONS,
                MAX_BROWSER_MAX_SESSIONS,
            )
        {
            issues.push(*issue);
        }
        if let Some(max_links) = self.delegate.child_runtime.browser.max_links
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.browser.max_links",
                max_links,
                MIN_BROWSER_MAX_LINKS,
                MAX_BROWSER_MAX_LINKS,
            )
        {
            issues.push(*issue);
        }
        if let Some(max_text_chars) = self.delegate.child_runtime.browser.max_text_chars
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.browser.max_text_chars",
                max_text_chars,
                MIN_BROWSER_MAX_TEXT_CHARS,
                MAX_BROWSER_MAX_TEXT_CHARS,
            )
        {
            issues.push(*issue);
        }
        if let Some(max_bytes) = self.delegate.child_runtime.web.max_bytes
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.web.max_bytes",
                max_bytes,
                MIN_WEB_FETCH_MAX_BYTES,
                MAX_WEB_FETCH_MAX_BYTES,
            )
        {
            issues.push(*issue);
        }
        if let Some(timeout_seconds) = self.delegate.child_runtime.web.timeout_seconds
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.web.timeout_seconds",
                timeout_seconds as usize,
                MIN_WEB_FETCH_TIMEOUT_SECONDS,
                MAX_WEB_FETCH_TIMEOUT_SECONDS,
            )
        {
            issues.push(*issue);
        }
        if let Some(max_redirects) = self.delegate.child_runtime.web.max_redirects
            && let Err(issue) = validate_numeric_range(
                "tools.delegate.child_runtime.web.max_redirects",
                max_redirects,
                0,
                MAX_WEB_FETCH_MAX_REDIRECTS,
            )
        {
            issues.push(*issue);
        }
        let timeout_as_usize = usize::try_from(self.web_search.timeout_seconds).map_err(|_e| {
            let mut vars = std::collections::BTreeMap::new();
            vars.insert(
                "actual_value".to_owned(),
                self.web_search.timeout_seconds.to_string(),
            );
            vars.insert("min".to_owned(), MIN_WEB_SEARCH_TIMEOUT_SECONDS.to_string());
            vars.insert("max".to_owned(), MAX_WEB_SEARCH_TIMEOUT_SECONDS.to_string());
            Box::new(super::shared::ConfigValidationIssue {
                severity: super::shared::ConfigValidationSeverity::Error,
                code: super::shared::ConfigValidationCode::NumericRange,
                field_path: "tools.web_search.timeout_seconds".to_owned(),
                inline_field_path: "tools.web_search.timeout_seconds".to_owned(),
                example_env_name: "LOONGCLAW_WEB_SEARCH_TIMEOUT_SECONDS".to_owned(),
                suggested_env_name: Some("LOONGCLAW_WEB_SEARCH_TIMEOUT_SECONDS".to_owned()),
                extra_message_variables: vars,
            })
        });
        match timeout_as_usize {
            Ok(v) => {
                if let Err(issue) = validate_numeric_range(
                    "tools.web_search.timeout_seconds",
                    v,
                    MIN_WEB_SEARCH_TIMEOUT_SECONDS,
                    MAX_WEB_SEARCH_TIMEOUT_SECONDS,
                ) {
                    issues.push(*issue);
                }
            }
            Err(issue) => issues.push(*issue),
        }
        if let Err(issue) = validate_numeric_range(
            "tools.web_search.max_results",
            self.web_search.max_results,
            MIN_WEB_SEARCH_MAX_RESULTS,
            MAX_WEB_SEARCH_MAX_RESULTS,
        ) {
            issues.push(*issue);
        }
        // Only validate provider settings when web_search is enabled
        // Note: API key validation is deferred to runtime since keys can be set via env vars
        if self.web_search.enabled
            && normalize_web_search_provider(self.web_search.default_provider.as_str()).is_none()
        {
            let mut extra_message_variables = std::collections::BTreeMap::new();
            extra_message_variables.insert(
                "provider_value".to_owned(),
                self.web_search.default_provider.clone(),
            );
            extra_message_variables.insert(
                "valid_providers".to_owned(),
                WEB_SEARCH_PROVIDER_VALID_VALUES.to_owned(),
            );
            issues.push(ConfigValidationIssue {
                severity: super::shared::ConfigValidationSeverity::Error,
                code: super::shared::ConfigValidationCode::UnknownSearchProvider,
                field_path: "tools.web_search.default_provider".to_owned(),
                inline_field_path: "tools.web_search.default_provider".to_owned(),
                example_env_name: "LOONGCLAW_WEB_SEARCH_PROVIDER".to_owned(),
                suggested_env_name: Some("LOONGCLAW_WEB_SEARCH_PROVIDER".to_owned()),
                extra_message_variables,
            });
        }
        issues
    }
}

pub fn normalize_web_search_provider(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "duckduckgo" | "ddg" => Some(WEB_SEARCH_PROVIDER_DUCKDUCKGO),
        "brave" => Some(WEB_SEARCH_PROVIDER_BRAVE),
        "tavily" => Some(WEB_SEARCH_PROVIDER_TAVILY),
        "perplexity" | "perplexity_search" => Some(WEB_SEARCH_PROVIDER_PERPLEXITY),
        "exa" => Some(WEB_SEARCH_PROVIDER_EXA),
        "jina" | "jinaai" | "jina-ai" => Some(WEB_SEARCH_PROVIDER_JINA),
        _ => None,
    }
}

pub fn web_search_provider_descriptors() -> &'static [WebSearchProviderDescriptor] {
    WEB_SEARCH_PROVIDER_DESCRIPTORS
}

pub fn web_search_provider_descriptor(raw: &str) -> Option<&'static WebSearchProviderDescriptor> {
    let normalized = normalize_web_search_provider(raw)?;
    WEB_SEARCH_PROVIDER_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == normalized)
}

pub fn web_search_provider_default_api_key_env(raw: &str) -> Option<&'static str> {
    web_search_provider_descriptor(raw).and_then(|descriptor| descriptor.default_api_key_env)
}

pub fn web_search_provider_api_key_env_names(raw: &str) -> &'static [&'static str] {
    web_search_provider_descriptor(raw)
        .map(|descriptor| descriptor.api_key_env_names)
        .unwrap_or(WEB_SEARCH_EMPTY_API_KEY_ENV_NAMES)
}

pub fn parse_autonomy_profile(raw: &str) -> Option<AutonomyProfile> {
    let normalized = raw.trim().to_ascii_lowercase();
    let matched_index = AUTONOMY_PROFILE_IDS
        .iter()
        .position(|value| *value == normalized)?;

    AutonomyProfile::from_index(matched_index)
}

#[cfg(feature = "tool-websearch")]
pub(crate) fn web_search_provider_parameter_description() -> String {
    format!(
        "Search provider. Defaults to '{DEFAULT_WEB_SEARCH_PROVIDER}'. Supported providers: {WEB_SEARCH_PROVIDER_VALID_VALUES}. DuckDuckGo works without a key. Brave, Tavily, Perplexity, Exa, and Jina use tools.web_search.brave_api_key / tools.web_search.tavily_api_key / tools.web_search.perplexity_api_key / tools.web_search.exa_api_key / tools.web_search.jina_api_key or the {WEB_SEARCH_BRAVE_API_KEY_ENV} / {WEB_SEARCH_TAVILY_API_KEY_ENV} / {WEB_SEARCH_PERPLEXITY_API_KEY_ENV} / {WEB_SEARCH_EXA_API_KEY_ENV} / {WEB_SEARCH_JINA_API_KEY_ENV} / {WEB_SEARCH_JINA_AUTH_TOKEN_ENV} environment variable fallbacks."
    )
}

impl BrowserCompanionToolConfig {
    pub fn normalized_allowed_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.allowed_domains)
    }

    pub fn normalized_blocked_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.blocked_domains)
    }
}

impl ExternalSkillsConfig {
    pub fn normalized_allowed_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.allowed_domains)
    }

    pub fn normalized_blocked_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.blocked_domains)
    }

    pub fn resolved_install_root(&self) -> Option<PathBuf> {
        self.install_root.as_deref().map(expand_path)
    }
}

impl WebToolConfig {
    pub fn normalized_allowed_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.allowed_domains)
    }

    pub fn normalized_blocked_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.blocked_domains)
    }
}

impl DelegateChildWebRuntimeConfig {
    pub fn normalized_allowed_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.allowed_domains)
    }

    pub fn normalized_blocked_domains(&self) -> Vec<String> {
        normalize_domain_entries(&self.blocked_domains)
    }
}

impl DelegateChildRuntimeConfig {
    pub fn runtime_narrowing(&self) -> crate::tools::runtime_config::ToolRuntimeNarrowing {
        crate::tools::runtime_config::ToolRuntimeNarrowing {
            web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                allow_private_hosts: self.web.allow_private_hosts,
                enforce_allowed_domains: !self.web.normalized_allowed_domains().is_empty(),
                allowed_domains: self.web.normalized_allowed_domains().into_iter().collect(),
                blocked_domains: self.web.normalized_blocked_domains().into_iter().collect(),
                timeout_seconds: self.web.timeout_seconds,
                max_bytes: self.web.max_bytes,
                max_redirects: self.web.max_redirects,
            },
            browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                max_sessions: self.browser.max_sessions,
                max_links: self.browser.max_links,
                max_text_chars: self.browser.max_text_chars,
            },
        }
    }
}

const fn default_enabled() -> bool {
    true
}

const fn default_session_list_limit() -> usize {
    100
}

const fn default_session_history_limit() -> usize {
    200
}

const fn default_delegate_max_depth() -> usize {
    1
}

const fn default_delegate_max_active_children() -> usize {
    5
}

const fn default_delegate_timeout_seconds() -> u64 {
    60
}

const fn default_browser_max_sessions() -> usize {
    DEFAULT_BROWSER_MAX_SESSIONS
}

const fn default_browser_max_links() -> usize {
    DEFAULT_BROWSER_MAX_LINKS
}

const fn default_browser_max_text_chars() -> usize {
    DEFAULT_BROWSER_MAX_TEXT_CHARS
}

fn default_delegate_child_tool_allowlist() -> Vec<String> {
    vec![
        "file.read".to_owned(),
        "file.write".to_owned(),
        "file.edit".to_owned(),
    ]
}

const fn default_web_fetch_max_bytes() -> usize {
    DEFAULT_WEB_FETCH_MAX_BYTES
}

const fn default_web_fetch_timeout_seconds() -> u64 {
    DEFAULT_WEB_FETCH_TIMEOUT_SECONDS
}

const fn default_web_fetch_max_redirects() -> usize {
    DEFAULT_WEB_FETCH_MAX_REDIRECTS
}

fn default_web_search_provider() -> String {
    DEFAULT_WEB_SEARCH_PROVIDER.to_owned()
}

const fn default_web_search_timeout_seconds() -> u64 {
    DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
}

const fn default_web_search_max_results() -> usize {
    DEFAULT_WEB_SEARCH_MAX_RESULTS
}

const fn default_require_download_approval() -> bool {
    true
}

const fn default_auto_expose_installed() -> bool {
    false
}

fn normalize_domain_entries(entries: &[String]) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    for entry in entries {
        let value = entry.trim().to_ascii_lowercase();
        if !value.is_empty() {
            normalized.insert(value);
        }
    }
    normalized.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_config_defaults_expose_session_runtime_policy() {
        let config = ToolConfig::default();
        assert!(config.shell_allow.is_empty());
        assert!(config.shell_deny.is_empty());
        assert_eq!(config.shell_default_mode, "deny");
        assert_eq!(config.autonomy_profile, AutonomyProfile::DiscoveryOnly);
        assert_eq!(config.consent.default_mode, ToolConsentMode::Full);
        assert_eq!(config.approval.mode, GovernedToolApprovalMode::Disabled);
        assert!(config.approval.approved_calls.is_empty());
        assert!(config.approval.denied_calls.is_empty());
        assert!(config.sessions.enabled);
        assert_eq!(config.sessions.visibility, SessionVisibility::Children);
        assert_eq!(config.sessions.list_limit, 100);
        assert_eq!(config.sessions.history_limit, 200);
        assert!(!config.sessions.allow_mutation);
        assert!(!config.messages.enabled);
        assert!(config.delegate.enabled);
        assert_eq!(config.delegate.max_depth, 1);
        assert_eq!(config.delegate.max_active_children, 5);
        assert_eq!(config.delegate.timeout_seconds, 60);
        assert_eq!(
            config.delegate.child_tool_allowlist,
            vec![
                "file.read".to_owned(),
                "file.write".to_owned(),
                "file.edit".to_owned()
            ]
        );
        assert!(!config.delegate.allow_shell_in_child);
        assert_eq!(
            config.runtime_self.max_source_chars,
            DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS
        );
        assert_eq!(
            config.runtime_self.max_total_chars,
            DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS
        );
        assert!(config.browser.enabled);
        assert_eq!(config.browser.max_sessions, 8);
        assert_eq!(config.browser.max_links, 40);
        assert_eq!(config.browser.max_text_chars, 6000);
        assert!(!config.browser_companion.enabled);
        assert!(config.browser_companion.command.is_none());
        assert!(config.browser_companion.expected_version.is_none());
        assert_eq!(
            config.browser_companion.timeout_seconds,
            DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS
        );
        assert!(config.web.enabled);
        assert!(!config.web.allow_private_hosts);
        assert!(config.web.allowed_domains.is_empty());
        assert!(config.web.blocked_domains.is_empty());
        assert_eq!(config.web.timeout_seconds, 15);
        assert_eq!(config.web.max_bytes, 1_048_576);
        assert_eq!(config.web.max_redirects, 3);
        // web_search defaults
        assert!(config.web_search.enabled);
        assert_eq!(
            config.web_search.default_provider,
            DEFAULT_WEB_SEARCH_PROVIDER
        );
        assert_eq!(
            config.web_search.timeout_seconds,
            DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
        );
        assert_eq!(
            config.web_search.max_results,
            DEFAULT_WEB_SEARCH_MAX_RESULTS
        );
        assert!(config.web_search.brave_api_key.is_none());
        assert!(config.web_search.tavily_api_key.is_none());
        assert!(config.web_search.perplexity_api_key.is_none());
        assert!(config.web_search.exa_api_key.is_none());
        assert!(config.web_search.jina_api_key.is_none());
    }

    #[test]
    fn parse_autonomy_profile_accepts_known_values() {
        assert_eq!(
            parse_autonomy_profile("discovery_only"),
            Some(AutonomyProfile::DiscoveryOnly)
        );
        assert_eq!(
            parse_autonomy_profile(" guided_acquisition "),
            Some(AutonomyProfile::GuidedAcquisition)
        );
        assert_eq!(
            parse_autonomy_profile("BOUNDED_AUTONOMOUS"),
            Some(AutonomyProfile::BoundedAutonomous)
        );
        assert_eq!(parse_autonomy_profile("unknown"), None);
    }

    #[test]
    fn autonomy_profile_valid_values_stays_in_sync_with_profile_ids() {
        let valid_values = [
            AutonomyProfile::DiscoveryOnly.as_str(),
            AutonomyProfile::GuidedAcquisition.as_str(),
            AutonomyProfile::BoundedAutonomous.as_str(),
        ]
        .join(", ");

        assert_eq!(AUTONOMY_PROFILE_VALID_VALUES, valid_values);
    }

    #[test]
    fn normalize_web_search_provider_canonicalizes_aliases() {
        assert_eq!(
            normalize_web_search_provider("duckduckgo"),
            Some(WEB_SEARCH_PROVIDER_DUCKDUCKGO)
        );
        assert_eq!(
            normalize_web_search_provider(" DDG "),
            Some(WEB_SEARCH_PROVIDER_DUCKDUCKGO)
        );
        assert_eq!(normalize_web_search_provider("brave"), Some("brave"));
        assert_eq!(normalize_web_search_provider("tavily"), Some("tavily"));
        assert_eq!(
            normalize_web_search_provider("perplexity"),
            Some("perplexity")
        );
        assert_eq!(normalize_web_search_provider("exa"), Some("exa"));
        assert_eq!(normalize_web_search_provider("jina-ai"), Some("jina"));
        assert_eq!(normalize_web_search_provider("unknown"), None);
        assert_eq!(DEFAULT_WEB_SEARCH_PROVIDER, WEB_SEARCH_PROVIDER_DUCKDUCKGO);
        assert!(WEB_SEARCH_PROVIDER_SCHEMA_VALUES.contains(&"perplexity_search"));
        assert!(WEB_SEARCH_PROVIDER_SCHEMA_VALUES.contains(&"jinaai"));
        assert!(WEB_SEARCH_PROVIDER_SCHEMA_VALUES.contains(&"jina-ai"));
        assert!(WEB_SEARCH_PROVIDER_VALID_VALUES.contains("perplexity_search"));
        assert!(WEB_SEARCH_PROVIDER_VALID_VALUES.contains("jinaai / jina-ai"));
    }

    #[test]
    fn web_search_provider_descriptor_reports_metadata() {
        let ddg = web_search_provider_descriptor("ddg").expect("duckduckgo descriptor");
        assert_eq!(ddg.id, WEB_SEARCH_PROVIDER_DUCKDUCKGO);
        assert_eq!(ddg.display_name, "DuckDuckGo");
        assert!(!ddg.requires_api_key);

        let tavily = web_search_provider_descriptor("tavily").expect("tavily descriptor");
        assert_eq!(
            tavily.default_api_key_env,
            Some(WEB_SEARCH_TAVILY_API_KEY_ENV)
        );

        let jina = web_search_provider_descriptor("jina").expect("jina descriptor");
        assert_eq!(jina.api_key_env_names, WEB_SEARCH_JINA_API_KEY_ENV_NAMES);
    }

    #[cfg(feature = "tool-websearch")]
    #[test]
    fn web_search_provider_parameter_description_mentions_config_and_env_fallbacks() {
        let description = web_search_provider_parameter_description();

        assert!(description.contains("tools.web_search.brave_api_key"));
        assert!(description.contains("tools.web_search.tavily_api_key"));
        assert!(description.contains("tools.web_search.perplexity_api_key"));
        assert!(description.contains("tools.web_search.exa_api_key"));
        assert!(description.contains("tools.web_search.jina_api_key"));
        assert!(description.contains(WEB_SEARCH_BRAVE_API_KEY_ENV));
        assert!(description.contains(WEB_SEARCH_TAVILY_API_KEY_ENV));
        assert!(description.contains(WEB_SEARCH_PERPLEXITY_API_KEY_ENV));
        assert!(description.contains(WEB_SEARCH_EXA_API_KEY_ENV));
        assert!(description.contains(WEB_SEARCH_JINA_API_KEY_ENV));
        assert!(description.contains(WEB_SEARCH_JINA_AUTH_TOKEN_ENV));
        assert!(description.contains(DEFAULT_WEB_SEARCH_PROVIDER));
        assert!(description.contains(WEB_SEARCH_PROVIDER_VALID_VALUES));
    }

    #[test]
    fn validate_rejects_web_search_timeout_below_minimum() {
        let mut config = ToolConfig::default();
        config.web_search.timeout_seconds = 0;

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.web_search.timeout_seconds"),
            "expected timeout_seconds validation issue, got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_web_search_timeout_above_maximum() {
        let mut config = ToolConfig::default();
        config.web_search.timeout_seconds = 61;

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.web_search.timeout_seconds"),
            "expected timeout_seconds validation issue, got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_web_search_max_results_out_of_range() {
        let mut config = ToolConfig::default();
        config.web_search.max_results = 0;
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.web_search.max_results"),
            "expected max_results validation issue, got {issues:?}"
        );

        config.web_search.max_results = 11;
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.web_search.max_results"),
            "expected max_results validation issue, got {issues:?}"
        );
    }

    #[test]
    fn validate_accepts_web_search_boundaries_and_alias_provider() {
        let mut config = ToolConfig::default();
        config.web_search.timeout_seconds = MIN_WEB_SEARCH_TIMEOUT_SECONDS as u64;
        config.web_search.max_results = MAX_WEB_SEARCH_MAX_RESULTS;
        config.web_search.default_provider = "ddg".to_owned();

        let issues = config.validate();

        assert!(
            issues.iter().all(|issue| {
                !matches!(
                    issue.field_path.as_str(),
                    "tools.web_search.timeout_seconds"
                        | "tools.web_search.max_results"
                        | "tools.web_search.default_provider"
                )
            }),
            "unexpected web_search validation issues: {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_zero_tool_execution_default_timeout() {
        let mut config = ToolConfig::default();
        config.tool_execution.default_timeout_seconds = Some(0);

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.tool_execution.default_timeout_seconds"),
            "expected default timeout validation issue, got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_zero_tool_execution_per_tool_timeout() {
        let mut config = ToolConfig::default();
        config
            .tool_execution
            .per_tool_timeout
            .insert("file.read".to_owned(), 0);

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "tools.tool_execution.per_tool_timeout.file.read"),
            "expected per-tool timeout validation issue, got {issues:?}"
        );
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_tool_consent_mode_from_toml() {
        let test_cases = [
            ("prompt", ToolConsentMode::Prompt),
            ("auto", ToolConsentMode::Auto),
            ("full", ToolConsentMode::Full),
        ];

        for (raw_mode, expected_mode) in test_cases {
            let raw = format!(
                r#"
[tools.consent]
default_mode = "{raw_mode}"
"#
            );
            let parsed =
                toml::from_str::<crate::config::LoongClawConfig>(&raw).expect("parse tool config");

            assert_eq!(parsed.tools.consent.default_mode, expected_mode);
        }
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_tool_execution_settings_from_toml() {
        let raw = r#"
[tools.tool_execution]
default_timeout_seconds = 12
per_tool_timeout = { "file.read" = 3, "web.search" = 9 }
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert_eq!(
            parsed.tools.tool_execution.default_timeout_seconds,
            Some(12)
        );
        assert_eq!(
            parsed
                .tools
                .tool_execution
                .per_tool_timeout
                .get("file.read"),
            Some(&3)
        );
        assert_eq!(
            parsed
                .tools
                .tool_execution
                .per_tool_timeout
                .get("web.search"),
            Some(&9)
        );
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_autonomy_profile_from_toml() {
        let raw = r#"
[tools]
autonomy_profile = "guided_acquisition"
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert_eq!(
            parsed.tools.autonomy_profile,
            AutonomyProfile::GuidedAcquisition
        );
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_session_runtime_controls_from_toml() {
        let raw = r#"
[tools.approval]
mode = "strict"
approved_calls = ["tool:delegate_async"]
denied_calls = ["tool:session_cancel"]

[tools.sessions]
visibility = "self"
list_limit = 12
history_limit = 34
allow_mutation = true

[tools.messages]
enabled = true

[tools.delegate]
enabled = false
max_depth = 2
max_active_children = 4
timeout_seconds = 90
allow_shell_in_child = true
child_tool_allowlist = ["file.read", "shell.exec"]

[tools.delegate.child_runtime.web]
allow_private_hosts = false
allowed_domains = ["Docs.Example.com", "docs.example.com"]
blocked_domains = ["internal.example", " INTERNAL.EXAMPLE "]
timeout_seconds = 9
max_bytes = 262144
max_redirects = 1

[tools.delegate.child_runtime.browser]
max_sessions = 2
max_links = 10
max_text_chars = 1024
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert_eq!(parsed.tools.approval.mode, GovernedToolApprovalMode::Strict);
        assert_eq!(
            parsed.tools.approval.approved_calls,
            vec!["tool:delegate_async".to_owned()]
        );
        assert_eq!(
            parsed.tools.approval.denied_calls,
            vec!["tool:session_cancel".to_owned()]
        );
        assert_eq!(
            parsed.tools.sessions.visibility,
            SessionVisibility::SelfOnly
        );
        assert_eq!(parsed.tools.sessions.list_limit, 12);
        assert_eq!(parsed.tools.sessions.history_limit, 34);
        assert!(parsed.tools.sessions.allow_mutation);
        assert!(parsed.tools.messages.enabled);
        assert!(!parsed.tools.delegate.enabled);
        assert_eq!(parsed.tools.delegate.max_depth, 2);
        assert_eq!(parsed.tools.delegate.max_active_children, 4);
        assert_eq!(parsed.tools.delegate.timeout_seconds, 90);
        assert!(parsed.tools.delegate.allow_shell_in_child);
        assert_eq!(
            parsed.tools.delegate.child_tool_allowlist,
            vec!["file.read".to_owned(), "shell.exec".to_owned()]
        );
        assert_eq!(
            parsed
                .tools
                .delegate
                .child_runtime
                .web
                .normalized_allowed_domains(),
            vec!["docs.example.com".to_owned()]
        );
        assert_eq!(
            parsed
                .tools
                .delegate
                .child_runtime
                .web
                .normalized_blocked_domains(),
            vec!["internal.example".to_owned()]
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.web.allow_private_hosts,
            Some(false)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.web.timeout_seconds,
            Some(9)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.web.max_bytes,
            Some(262144)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.web.max_redirects,
            Some(1)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.browser.max_sessions,
            Some(2)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.browser.max_links,
            Some(10)
        );
        assert_eq!(
            parsed.tools.delegate.child_runtime.browser.max_text_chars,
            Some(1024)
        );
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_runtime_self_controls_from_toml() {
        let raw = r#"
[tools.runtime_self]
max_source_chars = 12345
max_total_chars = 67890
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert_eq!(parsed.tools.runtime_self.max_source_chars, 12345);
        assert_eq!(parsed.tools.runtime_self.max_total_chars, 67890);
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_web_fetch_controls_from_toml() {
        let raw = r#"
[tools.web]
enabled = false
allow_private_hosts = true
allowed_domains = ["Docs.Example.com", "docs.example.com"]
blocked_domains = ["internal.example", " INTERNAL.EXAMPLE "]
timeout_seconds = 9
max_bytes = 262144
max_redirects = 1
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert!(!parsed.tools.web.enabled);
        assert!(parsed.tools.web.allow_private_hosts);
        assert_eq!(
            parsed.tools.web.normalized_allowed_domains(),
            vec!["docs.example.com".to_owned()]
        );
        assert_eq!(
            parsed.tools.web.normalized_blocked_domains(),
            vec!["internal.example".to_owned()]
        );
        assert_eq!(parsed.tools.web.timeout_seconds, 9);
        assert_eq!(parsed.tools.web.max_bytes, 262144);
        assert_eq!(parsed.tools.web.max_redirects, 1);
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_browser_controls_from_toml() {
        let raw = r#"
[tools.browser]
enabled = false
max_sessions = 4
max_links = 12
max_text_chars = 2048
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert!(!parsed.tools.browser.enabled);
        assert_eq!(parsed.tools.browser.max_sessions, 4);
        assert_eq!(parsed.tools.browser.max_links, 12);
        assert_eq!(parsed.tools.browser.max_text_chars, 2048);
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn tool_config_parses_browser_companion_controls_from_toml() {
        let raw = r#"
[tools.browser_companion]
enabled = true
command = "loongclaw-browser-companion"
expected_version = "1.2.3"
timeout_seconds = 7
allow_private_hosts = true
allowed_domains = ["Docs.Example.com", "docs.example.com", " api.example.com "]
blocked_domains = ["internal.example", " INTERNAL.EXAMPLE "]
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");

        assert!(parsed.tools.browser_companion.enabled);
        assert_eq!(
            parsed.tools.browser_companion.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(
            parsed.tools.browser_companion.expected_version.as_deref(),
            Some("1.2.3")
        );
        assert_eq!(parsed.tools.browser_companion.timeout_seconds, 7);
        assert!(parsed.tools.browser_companion.allow_private_hosts);
        assert_eq!(
            parsed.tools.browser_companion.normalized_allowed_domains(),
            vec!["api.example.com".to_owned(), "docs.example.com".to_owned()]
        );
        assert_eq!(
            parsed.tools.browser_companion.normalized_blocked_domains(),
            vec!["internal.example".to_owned()]
        );
    }

    #[test]
    fn browser_companion_defaults_to_safe_public_mode() {
        let config = BrowserCompanionToolConfig::default();
        assert!(!config.enabled);
        assert!(!config.allow_private_hosts);
        assert!(config.allowed_domains.is_empty());
        assert!(config.blocked_domains.is_empty());
        assert_eq!(
            config.timeout_seconds,
            default_browser_companion_timeout_seconds()
        );
    }

    /// When `shell_deny` is absent, it must default to empty — users start
    /// with no blocked commands beyond the default-deny fallback.
    #[test]
    #[cfg(feature = "config-toml")]
    fn tool_config_deny_defaults_to_empty() {
        let config: ToolConfig = toml::from_str("").expect("empty toml");
        assert!(config.shell_deny.is_empty());
    }

    /// An explicit `shell_allow = []` in the config file must produce an empty
    /// list, even though the serde default is non-empty.
    #[test]
    #[cfg(feature = "config-toml")]
    fn tool_config_explicit_empty_shell_allow_is_respected() {
        let config: ToolConfig = toml::from_str("shell_allow = []").expect("toml with empty allow");
        assert!(config.shell_allow.is_empty());
    }

    /// An explicit `shell_allow` with custom values replaces the defaults
    /// entirely; none of the 4 initial commands are injected.
    #[test]
    #[cfg(feature = "config-toml")]
    fn tool_config_explicit_shell_allow_replaces_defaults() {
        let config: ToolConfig = toml::from_str(r#"shell_allow = ["myapp"]"#).expect("toml");
        assert_eq!(config.shell_allow, vec!["myapp"]);
    }

    #[test]
    fn external_skills_defaults_to_safe_off_mode() {
        let config = ExternalSkillsConfig::default();
        assert!(!config.enabled);
        assert!(config.require_download_approval);
        assert!(config.allowed_domains.is_empty());
        assert!(config.blocked_domains.is_empty());
        assert!(config.install_root.is_none());
        assert!(!config.auto_expose_installed);
    }

    #[test]
    fn external_skills_normalized_domains_are_lowercase_and_deduped() {
        let config = ExternalSkillsConfig {
            enabled: true,
            require_download_approval: true,
            allowed_domains: vec![
                "Skills.SH".to_owned(),
                "skills.sh".to_owned(),
                "  CLAWHUB.IO ".to_owned(),
            ],
            blocked_domains: vec![
                "Bad.Example".to_owned(),
                "bad.example".to_owned(),
                " ".to_owned(),
            ],
            install_root: Some("~/skills".to_owned()),
            auto_expose_installed: true,
        };
        assert_eq!(
            config.normalized_allowed_domains(),
            vec!["clawhub.io".to_owned(), "skills.sh".to_owned()]
        );
        assert_eq!(
            config.normalized_blocked_domains(),
            vec!["bad.example".to_owned()]
        );
    }

    #[test]
    fn external_skills_resolved_install_root_expands_user_home() {
        let config = ExternalSkillsConfig {
            install_root: Some("~/demo-skills".to_owned()),
            ..ExternalSkillsConfig::default()
        };

        assert!(
            config
                .resolved_install_root()
                .expect("install root should resolve")
                .ends_with("demo-skills")
        );
    }

    #[test]
    fn web_tool_defaults_to_safe_public_fetch_mode() {
        let config = WebToolConfig::default();
        assert!(config.enabled);
        assert!(!config.allow_private_hosts);
        assert!(config.allowed_domains.is_empty());
        assert!(config.blocked_domains.is_empty());
        assert_eq!(config.timeout_seconds, 15);
        assert_eq!(config.max_bytes, 1_048_576);
        assert_eq!(config.max_redirects, 3);
    }

    #[test]
    fn web_tool_normalized_domains_are_lowercase_and_deduped() {
        let config = WebToolConfig {
            enabled: true,
            allow_private_hosts: false,
            allowed_domains: vec![
                "Docs.Example.com".to_owned(),
                "docs.example.com".to_owned(),
                "  api.example.com ".to_owned(),
            ],
            blocked_domains: vec![
                "internal.example".to_owned(),
                " INTERNAL.EXAMPLE ".to_owned(),
            ],
            timeout_seconds: 15,
            max_bytes: 1_048_576,
            max_redirects: 3,
        };

        assert_eq!(
            config.normalized_allowed_domains(),
            vec!["api.example.com".to_owned(), "docs.example.com".to_owned()]
        );
        assert_eq!(
            config.normalized_blocked_domains(),
            vec!["internal.example".to_owned()]
        );
    }
}
