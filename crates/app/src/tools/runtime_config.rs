use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use loongclaw_contracts::{ExecutionSecurityTier, SecretRef};
use serde::{Deserialize, Serialize};

use super::shell_policy_ext::ShellPolicyDefault;
use crate::config::{AutonomyProfile, LoongClawConfig};
#[cfg(feature = "feishu-integration")]
use crate::config::{FeishuChannelConfig, FeishuIntegrationConfig};
#[cfg(feature = "feishu-integration")]
use crate::secrets::has_configured_secret_ref;
use crate::secrets::{SecretLookup, resolve_secret_lookup};

fn bool_is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserRuntimeNarrowing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sessions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_links: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_text_chars: Option<usize>,
}

impl BrowserRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max_sessions.is_none() && self.max_links.is_none() && self.max_text_chars.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebFetchRuntimeNarrowing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_private_hosts: Option<bool>,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub enforce_allowed_domains: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_domains: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub blocked_domains: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_redirects: Option<usize>,
}

impl WebFetchRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allow_private_hosts.is_none()
            && !self.enforces_allowed_domains()
            && self.allowed_domains.is_empty()
            && self.blocked_domains.is_empty()
            && self.timeout_seconds.is_none()
            && self.max_bytes.is_none()
            && self.max_redirects.is_none()
    }

    #[must_use]
    pub fn enforces_allowed_domains(&self) -> bool {
        self.enforce_allowed_domains || !self.allowed_domains.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolRuntimeNarrowing {
    #[serde(default)]
    pub browser: BrowserRuntimeNarrowing,
    #[serde(default)]
    pub web_fetch: WebFetchRuntimeNarrowing,
}

impl ToolRuntimeNarrowing {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.browser.is_empty() && self.web_fetch.is_empty()
    }

    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        if self.is_empty() {
            return other.clone();
        }
        if other.is_empty() {
            return self.clone();
        }

        let browser = BrowserRuntimeNarrowing {
            max_sessions: min_optional_limit(self.browser.max_sessions, other.browser.max_sessions),
            max_links: min_optional_limit(self.browser.max_links, other.browser.max_links),
            max_text_chars: min_optional_limit(
                self.browser.max_text_chars,
                other.browser.max_text_chars,
            ),
        };

        let left_enforces_allowed_domains = self.web_fetch.enforces_allowed_domains();
        let right_enforces_allowed_domains = other.web_fetch.enforces_allowed_domains();
        let mut allowed_domains = BTreeSet::new();
        let mut enforce_allowed_domains = false;

        if left_enforces_allowed_domains && right_enforces_allowed_domains {
            enforce_allowed_domains = true;
            let left_is_deny_all = self.web_fetch.allowed_domains.is_empty();
            let right_is_deny_all = other.web_fetch.allowed_domains.is_empty();
            if !left_is_deny_all && !right_is_deny_all {
                allowed_domains = self
                    .web_fetch
                    .allowed_domains
                    .intersection(&other.web_fetch.allowed_domains)
                    .cloned()
                    .collect();
            }
        } else if left_enforces_allowed_domains {
            enforce_allowed_domains = true;
            allowed_domains = self.web_fetch.allowed_domains.clone();
        } else if right_enforces_allowed_domains {
            enforce_allowed_domains = true;
            allowed_domains = other.web_fetch.allowed_domains.clone();
        }

        let allow_private_hosts = intersect_private_host_setting(
            self.web_fetch.allow_private_hosts,
            other.web_fetch.allow_private_hosts,
        );

        let blocked_domains = self
            .web_fetch
            .blocked_domains
            .union(&other.web_fetch.blocked_domains)
            .cloned()
            .collect();

        let web_fetch = WebFetchRuntimeNarrowing {
            allow_private_hosts,
            enforce_allowed_domains,
            allowed_domains,
            blocked_domains,
            timeout_seconds: min_optional_limit(
                self.web_fetch.timeout_seconds,
                other.web_fetch.timeout_seconds,
            ),
            max_bytes: min_optional_limit(self.web_fetch.max_bytes, other.web_fetch.max_bytes),
            max_redirects: min_optional_limit(
                self.web_fetch.max_redirects,
                other.web_fetch.max_redirects,
            ),
        };

        Self { browser, web_fetch }
    }
}

fn min_optional_limit<T>(left: Option<T>, right: Option<T>) -> Option<T>
where
    T: Ord + Copy,
{
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn intersect_private_host_setting(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    if left == Some(false) || right == Some(false) {
        return Some(false);
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillsRuntimePolicy {
    pub enabled: bool,
    pub require_download_approval: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
    pub install_root: Option<PathBuf>,
    pub auto_expose_installed: bool,
}

impl Default for ExternalSkillsRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRuntimePolicy {
    pub enabled: bool,
    pub max_sessions: usize,
    pub max_links: usize,
    pub max_text_chars: usize,
}

impl Default for BrowserRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_sessions: crate::config::DEFAULT_BROWSER_MAX_SESSIONS,
            max_links: crate::config::DEFAULT_BROWSER_MAX_LINKS,
            max_text_chars: crate::config::DEFAULT_BROWSER_MAX_TEXT_CHARS,
        }
    }
}

impl BrowserRuntimePolicy {
    #[must_use]
    pub const fn execution_security_tier(&self) -> ExecutionSecurityTier {
        let _ = self;
        ExecutionSecurityTier::Restricted
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCompanionRuntimePolicy {
    pub enabled: bool,
    pub ready: bool,
    pub command: Option<String>,
    pub expected_version: Option<String>,
    pub timeout_seconds: u64,
    pub allow_private_hosts: bool,
    pub enforce_allowed_domains: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
}

impl Default for BrowserCompanionRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ready: false,
            command: None,
            expected_version: None,
            timeout_seconds: crate::config::DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS,
            allow_private_hosts: false,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
        }
    }
}

impl BrowserCompanionRuntimePolicy {
    #[must_use]
    pub fn is_runtime_ready(&self) -> bool {
        self.enabled && self.ready && self.command.is_some()
    }

    #[must_use]
    pub fn execution_security_tier(&self) -> ExecutionSecurityTier {
        if self.is_runtime_ready() {
            ExecutionSecurityTier::Balanced
        } else {
            ExecutionSecurityTier::Restricted
        }
    }

    #[must_use]
    pub fn web_policy(&self) -> WebFetchRuntimePolicy {
        WebFetchRuntimePolicy {
            enabled: self.enabled,
            allow_private_hosts: self.allow_private_hosts,
            enforce_allowed_domains: self.enforce_allowed_domains,
            allowed_domains: self.allowed_domains.clone(),
            blocked_domains: self.blocked_domains.clone(),
            timeout_seconds: self.timeout_seconds,
            max_bytes: crate::config::DEFAULT_WEB_FETCH_MAX_BYTES,
            max_redirects: crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSelfRuntimePolicy {
    pub max_source_chars: usize,
    pub max_total_chars: usize,
}

impl RuntimeSelfRuntimePolicy {
    #[must_use]
    pub fn from_limits(max_source_chars: usize, max_total_chars: usize) -> Self {
        let clamped_max_source_chars = max_source_chars.clamp(
            crate::config::MIN_RUNTIME_SELF_MAX_SOURCE_CHARS,
            crate::config::MAX_RUNTIME_SELF_MAX_SOURCE_CHARS,
        );
        let clamped_max_total_chars = max_total_chars.clamp(
            crate::config::MIN_RUNTIME_SELF_MAX_TOTAL_CHARS,
            crate::config::MAX_RUNTIME_SELF_MAX_TOTAL_CHARS,
        );

        Self {
            max_source_chars: clamped_max_source_chars,
            max_total_chars: clamped_max_total_chars,
        }
    }
}

impl Default for RuntimeSelfRuntimePolicy {
    fn default() -> Self {
        Self::from_limits(
            crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS,
            crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyOperationMode {
    #[default]
    Deny,
    ApprovalRequired,
    Allow,
}

impl AutonomyOperationMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::ApprovalRequired => "approval_required",
            Self::Allow => "allow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AutonomyBudgetPolicy {
    pub max_capability_acquisitions_per_turn: usize,
    pub max_provider_switches_per_turn: usize,
    pub max_topology_mutations_per_turn: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyPolicySnapshot {
    pub profile: AutonomyProfile,
    pub capability_acquisition_mode: AutonomyOperationMode,
    pub provider_switch_mode: AutonomyOperationMode,
    pub topology_mutation_mode: AutonomyOperationMode,
    pub requires_kernel_binding: bool,
    pub budget: AutonomyBudgetPolicy,
}

impl AutonomyPolicySnapshot {
    #[must_use]
    pub fn from_profile(profile: AutonomyProfile) -> Self {
        match profile {
            AutonomyProfile::DiscoveryOnly => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::Deny,
                provider_switch_mode: AutonomyOperationMode::Deny,
                topology_mutation_mode: AutonomyOperationMode::Deny,
                requires_kernel_binding: false,
                budget: AutonomyBudgetPolicy::default(),
            },
            AutonomyProfile::GuidedAcquisition => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::ApprovalRequired,
                provider_switch_mode: AutonomyOperationMode::ApprovalRequired,
                topology_mutation_mode: AutonomyOperationMode::Deny,
                requires_kernel_binding: true,
                budget: AutonomyBudgetPolicy {
                    max_capability_acquisitions_per_turn: 1,
                    max_provider_switches_per_turn: 1,
                    max_topology_mutations_per_turn: 0,
                },
            },
            AutonomyProfile::BoundedAutonomous => Self {
                profile,
                capability_acquisition_mode: AutonomyOperationMode::Allow,
                provider_switch_mode: AutonomyOperationMode::ApprovalRequired,
                topology_mutation_mode: AutonomyOperationMode::Deny,
                requires_kernel_binding: true,
                budget: AutonomyBudgetPolicy {
                    max_capability_acquisitions_per_turn: 2,
                    max_provider_switches_per_turn: 1,
                    max_topology_mutations_per_turn: 0,
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchRuntimePolicy {
    pub enabled: bool,
    pub allow_private_hosts: bool,
    pub enforce_allowed_domains: bool,
    pub allowed_domains: BTreeSet<String>,
    pub blocked_domains: BTreeSet<String>,
    pub timeout_seconds: u64,
    pub max_bytes: usize,
    pub max_redirects: usize,
}

impl Default for WebFetchRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_hosts: false,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: crate::config::DEFAULT_WEB_FETCH_TIMEOUT_SECONDS,
            max_bytes: crate::config::DEFAULT_WEB_FETCH_MAX_BYTES,
            max_redirects: crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchRuntimePolicy {
    pub enabled: bool,
    pub default_provider: String,
    pub brave_api_key: Option<String>,
    pub tavily_api_key: Option<String>,
    pub perplexity_api_key: Option<String>,
    pub exa_api_key: Option<String>,
    pub jina_api_key: Option<String>,
    pub timeout_seconds: u64,
    pub max_results: usize,
}

impl Default for WebSearchRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            default_provider: crate::config::DEFAULT_WEB_SEARCH_PROVIDER.to_owned(),
            brave_api_key: None,
            tavily_api_key: None,
            perplexity_api_key: None,
            exa_api_key: None,
            jina_api_key: None,
            timeout_seconds: crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS,
            max_results: crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS,
        }
    }
}

#[cfg(feature = "feishu-integration")]
#[derive(Debug, Clone)]
pub struct FeishuToolRuntimeConfig {
    pub channel: FeishuChannelConfig,
    pub integration: FeishuIntegrationConfig,
}

#[cfg(feature = "feishu-integration")]
impl FeishuToolRuntimeConfig {
    pub fn from_loongclaw_config(config: &LoongClawConfig) -> Option<Self> {
        has_enabled_feishu_runtime_credentials(&config.feishu).then(|| Self {
            channel: config.feishu.clone(),
            integration: config.feishu_integration.clone(),
        })
    }

    fn from_env() -> Option<Self> {
        has_feishu_runtime_credentials(&FeishuChannelConfig::default()).then(|| Self {
            channel: FeishuChannelConfig {
                enabled: true,
                ..FeishuChannelConfig::default()
            },
            integration: FeishuIntegrationConfig::default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolExecutionConfig {
    pub default_timeout_seconds: Option<u64>,
    pub per_tool_timeout: BTreeMap<String, u64>,
}

impl ToolExecutionConfig {
    pub fn timeout_for_tool(&self, tool_name: &str) -> Option<u64> {
        let key = tool_name.to_lowercase();
        self.per_tool_timeout
            .get(&key)
            .copied()
            .or(self.default_timeout_seconds)
    }
}

/// Typed runtime configuration for tool executors.
///
/// Replaces per-call `std::env::var` lookups with a single read from a
/// process-wide singleton that is populated once at startup.
#[derive(Debug, Clone)]
pub struct ToolRuntimeConfig {
    pub file_root: Option<PathBuf>,
    pub shell_allow: BTreeSet<String>,
    pub shell_deny: BTreeSet<String>,
    pub shell_default_mode: ShellPolicyDefault,
    pub config_path: Option<PathBuf>,
    pub sessions_enabled: bool,
    pub sessions_allow_mutation: bool,
    pub messages_enabled: bool,
    pub delegate_enabled: bool,
    pub runtime_self: RuntimeSelfRuntimePolicy,
    pub browser: BrowserRuntimePolicy,
    pub browser_companion: BrowserCompanionRuntimePolicy,
    pub web_fetch: WebFetchRuntimePolicy,
    pub web_search: WebSearchRuntimePolicy,
    pub autonomy_profile: AutonomyProfile,
    pub external_skills: ExternalSkillsRuntimePolicy,
    pub tool_execution: ToolExecutionConfig,
    #[cfg(feature = "feishu-integration")]
    pub feishu: Option<FeishuToolRuntimeConfig>,
}

impl Default for ToolRuntimeConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            shell_allow: crate::config::DEFAULT_SHELL_ALLOW
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            shell_deny: BTreeSet::new(),
            shell_default_mode: ShellPolicyDefault::Deny,
            config_path: None,
            sessions_enabled: true,
            sessions_allow_mutation: false,
            messages_enabled: false,
            delegate_enabled: true,
            runtime_self: RuntimeSelfRuntimePolicy::default(),
            browser: BrowserRuntimePolicy::default(),
            browser_companion: BrowserCompanionRuntimePolicy::default(),
            web_fetch: WebFetchRuntimePolicy::default(),
            web_search: WebSearchRuntimePolicy::default(),
            autonomy_profile: AutonomyProfile::default(),
            external_skills: ExternalSkillsRuntimePolicy::default(),
            tool_execution: ToolExecutionConfig::default(),
            #[cfg(feature = "feishu-integration")]
            feishu: None,
        }
    }
}

impl ToolRuntimeConfig {
    pub fn from_loongclaw_config(config: &LoongClawConfig, config_path: Option<&Path>) -> Self {
        let web_fetch_allowed_domains = config.tools.web.normalized_allowed_domains();
        let web_fetch_enforce_allowed_domains = !web_fetch_allowed_domains.is_empty();
        let browser_companion_allowed_domains =
            config.tools.browser_companion.normalized_allowed_domains();
        let browser_companion_enforce_allowed_domains =
            !browser_companion_allowed_domains.is_empty();
        Self {
            file_root: Some(config.tools.resolved_file_root()),
            shell_allow: config
                .tools
                .shell_allow
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect(),
            shell_deny: config
                .tools
                .shell_deny
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect(),
            shell_default_mode: ShellPolicyDefault::parse(&config.tools.shell_default_mode),
            config_path: config_path.map(Path::to_path_buf),
            sessions_enabled: config.tools.sessions.enabled,
            sessions_allow_mutation: config.tools.sessions.allow_mutation,
            messages_enabled: config.tools.messages.enabled,
            delegate_enabled: config.tools.delegate.enabled,
            runtime_self: RuntimeSelfRuntimePolicy::from_limits(
                config.tools.runtime_self.max_source_chars,
                config.tools.runtime_self.max_total_chars,
            ),
            browser: BrowserRuntimePolicy {
                enabled: config.tools.browser.enabled,
                max_sessions: config.tools.browser.max_sessions,
                max_links: config.tools.browser.max_links,
                max_text_chars: config.tools.browser.max_text_chars,
            },
            browser_companion: browser_companion_runtime_policy(
                config.tools.browser_companion.enabled,
                parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false),
                config.tools.browser_companion.command.as_deref(),
                config.tools.browser_companion.expected_version.as_deref(),
                config.tools.browser_companion.timeout_seconds,
                config.tools.browser_companion.allow_private_hosts,
                browser_companion_allowed_domains.into_iter().collect(),
                config
                    .tools
                    .browser_companion
                    .normalized_blocked_domains()
                    .into_iter()
                    .collect(),
                browser_companion_enforce_allowed_domains,
            ),
            web_fetch: WebFetchRuntimePolicy {
                enabled: config.tools.web.enabled,
                allow_private_hosts: config.tools.web.allow_private_hosts,
                enforce_allowed_domains: web_fetch_enforce_allowed_domains,
                allowed_domains: web_fetch_allowed_domains.into_iter().collect(),
                blocked_domains: config
                    .tools
                    .web
                    .normalized_blocked_domains()
                    .into_iter()
                    .collect(),
                timeout_seconds: config.tools.web.timeout_seconds,
                max_bytes: config.tools.web.max_bytes,
                max_redirects: config.tools.web.max_redirects,
            },
            web_search: WebSearchRuntimePolicy {
                enabled: config.tools.web_search.enabled,
                default_provider: crate::config::normalize_web_search_provider(
                    config.tools.web_search.default_provider.as_str(),
                )
                .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_PROVIDER)
                .to_owned(),
                brave_api_key: resolve_web_search_secret_binding(
                    config.tools.web_search.brave_api_key.as_deref(),
                    crate::config::web_search_provider_api_key_env_names(
                        crate::config::WEB_SEARCH_PROVIDER_BRAVE,
                    ),
                ),
                tavily_api_key: resolve_web_search_secret_binding(
                    config.tools.web_search.tavily_api_key.as_deref(),
                    crate::config::web_search_provider_api_key_env_names(
                        crate::config::WEB_SEARCH_PROVIDER_TAVILY,
                    ),
                ),
                perplexity_api_key: resolve_web_search_secret_binding(
                    config.tools.web_search.perplexity_api_key.as_deref(),
                    crate::config::web_search_provider_api_key_env_names(
                        crate::config::WEB_SEARCH_PROVIDER_PERPLEXITY,
                    ),
                ),
                exa_api_key: resolve_web_search_secret_binding(
                    config.tools.web_search.exa_api_key.as_deref(),
                    crate::config::web_search_provider_api_key_env_names(
                        crate::config::WEB_SEARCH_PROVIDER_EXA,
                    ),
                ),
                jina_api_key: resolve_web_search_secret_binding(
                    config.tools.web_search.jina_api_key.as_deref(),
                    crate::config::web_search_provider_api_key_env_names(
                        crate::config::WEB_SEARCH_PROVIDER_JINA,
                    ),
                ),
                timeout_seconds: config.tools.web_search.timeout_seconds,
                max_results: config.tools.web_search.max_results,
            },
            autonomy_profile: config.tools.autonomy_profile,
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: config.external_skills.enabled,
                require_download_approval: config.external_skills.require_download_approval,
                allowed_domains: config
                    .external_skills
                    .normalized_allowed_domains()
                    .into_iter()
                    .collect(),
                blocked_domains: config
                    .external_skills
                    .normalized_blocked_domains()
                    .into_iter()
                    .collect(),
                install_root: config.external_skills.resolved_install_root(),
                auto_expose_installed: config.external_skills.auto_expose_installed,
            },
            tool_execution: ToolExecutionConfig {
                default_timeout_seconds: config.tools.tool_execution.default_timeout_seconds,
                per_tool_timeout: config
                    .tools
                    .tool_execution
                    .per_tool_timeout
                    .iter()
                    .map(|(k, v): (&String, &u64)| (k.to_lowercase(), *v))
                    .collect(),
            },
            #[cfg(feature = "feishu-integration")]
            feishu: FeishuToolRuntimeConfig::from_loongclaw_config(config),
        }
    }

    /// Build a config by reading the legacy environment variables.
    ///
    /// Keeps full backward compatibility for callers that still rely on
    /// `LOONGCLAW_FILE_ROOT`.
    pub fn from_env() -> Self {
        let file_root = std::env::var("LOONGCLAW_FILE_ROOT").ok().map(PathBuf::from);
        let config_path = std::env::var("LOONGCLAW_CONFIG_PATH")
            .ok()
            .map(PathBuf::from);
        let sessions_enabled = parse_env_bool("LOONGCLAW_TOOL_SESSIONS_ENABLED").unwrap_or(true);
        let sessions_allow_mutation =
            parse_env_bool("LOONGCLAW_TOOL_SESSIONS_ALLOW_MUTATION").unwrap_or(false);
        let messages_enabled = parse_env_bool("LOONGCLAW_TOOL_MESSAGES_ENABLED").unwrap_or(false);
        let delegate_enabled = parse_env_bool("LOONGCLAW_TOOL_DELEGATE_ENABLED").unwrap_or(true);
        let runtime_self_max_source_chars =
            parse_env_usize("LOONGCLAW_RUNTIME_SELF_MAX_SOURCE_CHARS")
                .unwrap_or(crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS);
        let runtime_self_max_total_chars =
            parse_env_usize("LOONGCLAW_RUNTIME_SELF_MAX_TOTAL_CHARS")
                .unwrap_or(crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS);
        let runtime_self_policy = RuntimeSelfRuntimePolicy::from_limits(
            runtime_self_max_source_chars,
            runtime_self_max_total_chars,
        );
        let browser_enabled = parse_env_bool("LOONGCLAW_BROWSER_ENABLED").unwrap_or(true);
        let browser_max_sessions = parse_env_usize("LOONGCLAW_BROWSER_MAX_SESSIONS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_SESSIONS);
        let browser_max_links = parse_env_usize("LOONGCLAW_BROWSER_MAX_LINKS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_LINKS);
        let browser_max_text_chars = parse_env_usize("LOONGCLAW_BROWSER_MAX_TEXT_CHARS")
            .unwrap_or(crate::config::DEFAULT_BROWSER_MAX_TEXT_CHARS);
        let browser_companion_enabled =
            parse_env_bool("LOONGCLAW_BROWSER_COMPANION_ENABLED").unwrap_or(false);
        let browser_companion_ready =
            parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false);
        let browser_companion_command = parse_env_string("LOONGCLAW_BROWSER_COMPANION_COMMAND");
        let browser_companion_expected_version =
            parse_env_string("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION");
        let browser_companion_timeout_seconds =
            parse_env_u64("LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS")
                .unwrap_or(crate::config::DEFAULT_BROWSER_COMPANION_TIMEOUT_SECONDS);
        let web_fetch_enabled = parse_env_bool("LOONGCLAW_WEB_FETCH_ENABLED").unwrap_or(true);
        let web_fetch_allow_private_hosts =
            parse_env_bool("LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS").unwrap_or(false);
        let web_fetch_allowed_domains =
            parse_env_domain_list("LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS");
        let web_fetch_blocked_domains =
            parse_env_domain_list("LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS");
        let web_fetch_timeout_seconds = parse_env_u64("LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_TIMEOUT_SECONDS);
        let web_fetch_max_bytes = parse_env_usize("LOONGCLAW_WEB_FETCH_MAX_BYTES")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_MAX_BYTES);
        let web_fetch_max_redirects = parse_env_usize("LOONGCLAW_WEB_FETCH_MAX_REDIRECTS")
            .unwrap_or(crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS);
        let web_search_enabled = parse_env_bool("LOONGCLAW_WEB_SEARCH_ENABLED").unwrap_or(true);
        let web_search_default_provider = parse_env_string("LOONGCLAW_WEB_SEARCH_PROVIDER")
            .as_deref()
            .and_then(crate::config::normalize_web_search_provider)
            .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_PROVIDER)
            .to_owned();
        let web_search_brave_api_key = resolve_web_search_secret_binding(
            None,
            crate::config::web_search_provider_api_key_env_names(
                crate::config::WEB_SEARCH_PROVIDER_BRAVE,
            ),
        );
        let web_search_tavily_api_key = resolve_web_search_secret_binding(
            None,
            crate::config::web_search_provider_api_key_env_names(
                crate::config::WEB_SEARCH_PROVIDER_TAVILY,
            ),
        );
        let web_search_perplexity_api_key = resolve_web_search_secret_binding(
            None,
            crate::config::web_search_provider_api_key_env_names(
                crate::config::WEB_SEARCH_PROVIDER_PERPLEXITY,
            ),
        );
        let web_search_exa_api_key = resolve_web_search_secret_binding(
            None,
            crate::config::web_search_provider_api_key_env_names(
                crate::config::WEB_SEARCH_PROVIDER_EXA,
            ),
        );
        let web_search_jina_api_key = resolve_web_search_secret_binding(
            None,
            crate::config::web_search_provider_api_key_env_names(
                crate::config::WEB_SEARCH_PROVIDER_JINA,
            ),
        );
        let web_search_timeout_seconds = parse_env_u64("LOONGCLAW_WEB_SEARCH_TIMEOUT_SECONDS")
            .map(|seconds| seconds.clamp(1, 60))
            .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS);
        let web_search_max_results = parse_env_usize("LOONGCLAW_WEB_SEARCH_MAX_RESULTS")
            .map(|count| count.clamp(1, 10))
            .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS);
        let autonomy_profile = resolve_autonomy_profile_from_env();
        let enabled = parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_ENABLED").unwrap_or(false);
        let require_download_approval =
            parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL").unwrap_or(true);
        let allowed_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        let blocked_domains = parse_env_domain_list("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");
        let install_root = std::env::var("LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT")
            .ok()
            .map(PathBuf::from);
        let auto_expose_installed =
            parse_env_bool("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED").unwrap_or(false);

        let tool_execution_default_timeout =
            parse_env_u64("LOONGCLAW_TOOL_DEFAULT_TIMEOUT_SECONDS");
        let mut tool_execution_per_tool_timeout = BTreeMap::new();
        for (key, value) in std::env::vars() {
            if let Some(tool_name) = key.strip_prefix("LOONGCLAW_TOOL_")
                && let Some(stripped) = tool_name.strip_suffix("_TIMEOUT_SECONDS")
                && !stripped.is_empty()
                && stripped != "DEFAULT"
                && let Ok(timeout) = value.parse::<u64>()
            {
                tool_execution_per_tool_timeout.insert(stripped.to_lowercase(), timeout);
            }
        }
        let tool_execution = ToolExecutionConfig {
            default_timeout_seconds: tool_execution_default_timeout,
            per_tool_timeout: tool_execution_per_tool_timeout,
        };

        Self {
            file_root,
            config_path,
            sessions_enabled,
            sessions_allow_mutation,
            messages_enabled,
            delegate_enabled,
            runtime_self: runtime_self_policy,
            browser: BrowserRuntimePolicy {
                enabled: browser_enabled,
                max_sessions: browser_max_sessions,
                max_links: browser_max_links,
                max_text_chars: browser_max_text_chars,
            },
            browser_companion: browser_companion_runtime_policy(
                browser_companion_enabled,
                browser_companion_ready,
                browser_companion_command.as_deref(),
                browser_companion_expected_version.as_deref(),
                browser_companion_timeout_seconds,
                false,
                BTreeSet::new(),
                BTreeSet::new(),
                false,
            ),
            web_fetch: WebFetchRuntimePolicy {
                enabled: web_fetch_enabled,
                allow_private_hosts: web_fetch_allow_private_hosts,
                enforce_allowed_domains: !web_fetch_allowed_domains.is_empty(),
                allowed_domains: web_fetch_allowed_domains,
                blocked_domains: web_fetch_blocked_domains,
                timeout_seconds: web_fetch_timeout_seconds,
                max_bytes: web_fetch_max_bytes,
                max_redirects: web_fetch_max_redirects,
            },
            web_search: WebSearchRuntimePolicy {
                enabled: web_search_enabled,
                default_provider: web_search_default_provider,
                brave_api_key: web_search_brave_api_key,
                tavily_api_key: web_search_tavily_api_key,
                perplexity_api_key: web_search_perplexity_api_key,
                exa_api_key: web_search_exa_api_key,
                jina_api_key: web_search_jina_api_key,
                timeout_seconds: web_search_timeout_seconds,
                max_results: web_search_max_results,
            },
            autonomy_profile,
            tool_execution,
            ..Self::default()
        }
        .with_external_skills_policy(ExternalSkillsRuntimePolicy {
            enabled,
            require_download_approval,
            allowed_domains,
            blocked_domains,
            install_root,
            auto_expose_installed,
        })
    }

    fn with_external_skills_policy(mut self, external_skills: ExternalSkillsRuntimePolicy) -> Self {
        self.external_skills = external_skills;
        #[cfg(feature = "feishu-integration")]
        {
            self.feishu = FeishuToolRuntimeConfig::from_env();
        }
        self
    }

    #[must_use]
    pub fn narrowed(&self, narrowing: &ToolRuntimeNarrowing) -> Self {
        if narrowing.is_empty() {
            return self.clone();
        }

        let mut narrowed = self.clone();

        if let Some(max_sessions) = narrowing.browser.max_sessions {
            narrowed.browser.max_sessions = narrowed.browser.max_sessions.min(max_sessions.max(1));
        }
        if let Some(max_links) = narrowing.browser.max_links {
            narrowed.browser.max_links = narrowed.browser.max_links.min(max_links.max(1));
        }
        if let Some(max_text_chars) = narrowing.browser.max_text_chars {
            narrowed.browser.max_text_chars =
                narrowed.browser.max_text_chars.min(max_text_chars.max(1));
        }

        narrowed.web_fetch.allow_private_hosts = match narrowing.web_fetch.allow_private_hosts {
            Some(false) => false,
            Some(true) => narrowed.web_fetch.allow_private_hosts,
            None => narrowed.web_fetch.allow_private_hosts,
        };

        let preserve_deny_all = narrowed.web_fetch.enforce_allowed_domains
            && narrowed.web_fetch.allowed_domains.is_empty();
        if narrowing.web_fetch.enforces_allowed_domains() {
            narrowed.web_fetch.enforce_allowed_domains = true;
            if !preserve_deny_all {
                narrowed.web_fetch.allowed_domains =
                    if narrowing.web_fetch.allowed_domains.is_empty() {
                        BTreeSet::new()
                    } else if narrowed.web_fetch.allowed_domains.is_empty() {
                        narrowing.web_fetch.allowed_domains.clone()
                    } else {
                        narrowed
                            .web_fetch
                            .allowed_domains
                            .intersection(&narrowing.web_fetch.allowed_domains)
                            .cloned()
                            .collect()
                    };
            }
        }
        narrowed
            .web_fetch
            .blocked_domains
            .extend(narrowing.web_fetch.blocked_domains.iter().cloned());

        if let Some(timeout_seconds) = narrowing.web_fetch.timeout_seconds {
            narrowed.web_fetch.timeout_seconds = narrowed
                .web_fetch
                .timeout_seconds
                .min(timeout_seconds.max(1));
        }
        if let Some(max_bytes) = narrowing.web_fetch.max_bytes {
            narrowed.web_fetch.max_bytes = narrowed.web_fetch.max_bytes.min(max_bytes.max(1));
        }
        if let Some(max_redirects) = narrowing.web_fetch.max_redirects {
            narrowed.web_fetch.max_redirects = narrowed.web_fetch.max_redirects.min(max_redirects);
        }

        narrowed
    }

    #[must_use]
    pub(crate) fn delegate_child_prompt_summary(
        &self,
        narrowing: &ToolRuntimeNarrowing,
    ) -> Option<String> {
        if narrowing.is_empty() {
            return None;
        }

        let effective = self.narrowed(narrowing);
        let mut lines = vec![
            "[delegate_child_runtime_contract]".to_owned(),
            "Plan within these child-session runtime limits:".to_owned(),
        ];
        let mut rendered_any = false;

        if effective.web_fetch.enabled {
            if narrowing.web_fetch.allow_private_hosts.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- web.fetch private hosts: {}",
                    if effective.web_fetch.allow_private_hosts {
                        "allowed"
                    } else {
                        "denied"
                    }
                ));
            }
            if narrowing.web_fetch.enforces_allowed_domains() {
                rendered_any = true;
                if effective.web_fetch.enforce_allowed_domains
                    && effective.web_fetch.allowed_domains.is_empty()
                {
                    lines.push(
                        "- web.fetch allowed domains: none (effective intersection is empty)"
                            .to_owned(),
                    );
                } else {
                    lines.push(format!(
                        "- web.fetch allowed domains: {}",
                        effective
                            .web_fetch
                            .allowed_domains
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if !narrowing.web_fetch.blocked_domains.is_empty() {
                rendered_any = true;
                lines.push(format!(
                    "- web.fetch blocked domains: {}",
                    effective
                        .web_fetch
                        .blocked_domains
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if narrowing.web_fetch.timeout_seconds.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- web.fetch timeout seconds: {}",
                    effective.web_fetch.timeout_seconds
                ));
            }
            if narrowing.web_fetch.max_bytes.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- web.fetch max bytes: {}",
                    effective.web_fetch.max_bytes
                ));
            }
            if narrowing.web_fetch.max_redirects.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- web.fetch max redirects: {}",
                    effective.web_fetch.max_redirects
                ));
            }
        }

        if effective.browser.enabled {
            if narrowing.browser.max_sessions.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max sessions: {}",
                    effective.browser.max_sessions
                ));
            }
            if narrowing.browser.max_links.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max links: {}",
                    effective.browser.max_links
                ));
            }
            if narrowing.browser.max_text_chars.is_some() {
                rendered_any = true;
                lines.push(format!(
                    "- browser max text chars: {}",
                    effective.browser.max_text_chars
                ));
            }
        }

        if !rendered_any {
            return None;
        }

        lines.push("Treat these as enforced limits for this child session.".to_owned());
        Some(lines.join("\n"))
    }

    #[must_use]
    pub const fn browser_execution_security_tier(&self) -> ExecutionSecurityTier {
        self.browser.execution_security_tier()
    }

    #[must_use]
    pub fn browser_companion_execution_security_tier(&self) -> ExecutionSecurityTier {
        self.browser_companion.execution_security_tier()
    }

    #[must_use]
    pub fn autonomy_policy_snapshot(&self) -> AutonomyPolicySnapshot {
        AutonomyPolicySnapshot::from_profile(self.autonomy_profile)
    }
}

fn resolve_web_search_secret_binding(
    configured_value: Option<&str>,
    env_names: &[&str],
) -> Option<String> {
    if let Some(secret_ref) = configured_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| SecretRef::Inline(value.to_owned()))
    {
        match resolve_secret_lookup(Some(&secret_ref)) {
            SecretLookup::Value(value) => return Some(value),
            SecretLookup::Missing => return None,
            SecretLookup::Absent => {}
        }
    }

    env_names
        .iter()
        .find_map(|env_name| std::env::var(env_name).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) fn browser_companion_runtime_policy_from_tool_config(
    config: &crate::config::ToolConfig,
) -> BrowserCompanionRuntimePolicy {
    let allowed_domains = config.browser_companion.normalized_allowed_domains();
    let enforce_allowed_domains = !allowed_domains.is_empty();
    browser_companion_runtime_policy(
        config.browser_companion.enabled,
        parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false),
        config.browser_companion.command.as_deref(),
        config.browser_companion.expected_version.as_deref(),
        config.browser_companion.timeout_seconds,
        config.browser_companion.allow_private_hosts,
        allowed_domains.into_iter().collect(),
        config
            .browser_companion
            .normalized_blocked_domains()
            .into_iter()
            .collect(),
        enforce_allowed_domains,
    )
}

pub(crate) fn browser_companion_runtime_policy_with_env_fallback(
    config: &crate::config::ToolConfig,
) -> BrowserCompanionRuntimePolicy {
    let env_command = parse_env_string("LOONGCLAW_BROWSER_COMPANION_COMMAND");
    let env_expected_version = parse_env_string("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION");
    let allowed_domains = config.browser_companion.normalized_allowed_domains();
    let enforce_allowed_domains = !allowed_domains.is_empty();
    browser_companion_runtime_policy(
        config.browser_companion.enabled
            || parse_env_bool("LOONGCLAW_BROWSER_COMPANION_ENABLED").unwrap_or(false),
        parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false),
        config
            .browser_companion
            .command
            .as_deref()
            .or(env_command.as_deref()),
        config
            .browser_companion
            .expected_version
            .as_deref()
            .or(env_expected_version.as_deref()),
        parse_env_u64("LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS")
            .unwrap_or(config.browser_companion.timeout_seconds),
        config.browser_companion.allow_private_hosts,
        allowed_domains.into_iter().collect(),
        config
            .browser_companion
            .normalized_blocked_domains()
            .into_iter()
            .collect(),
        enforce_allowed_domains,
    )
}

fn browser_companion_runtime_policy(
    enabled: bool,
    ready: bool,
    command: Option<&str>,
    expected_version: Option<&str>,
    timeout_seconds: u64,
    allow_private_hosts: bool,
    allowed_domains: BTreeSet<String>,
    blocked_domains: BTreeSet<String>,
    enforce_allowed_domains: bool,
) -> BrowserCompanionRuntimePolicy {
    BrowserCompanionRuntimePolicy {
        enabled,
        ready,
        command: normalize_optional_string(command),
        expected_version: normalize_optional_string(expected_version),
        timeout_seconds: timeout_seconds.max(1),
        allow_private_hosts,
        enforce_allowed_domains,
        allowed_domains,
        blocked_domains,
    }
}

fn parse_env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|raw| {
        let value = raw.trim().to_ascii_lowercase();
        match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn parse_env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn parse_env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
}

fn normalize_optional_string(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_env_string(key: &str) -> Option<String> {
    normalize_optional_string(std::env::var(key).ok().as_deref())
}

fn parse_env_domain_list(key: &str) -> BTreeSet<String> {
    std::env::var(key)
        .ok()
        .unwrap_or_default()
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn resolve_autonomy_profile_from_env() -> AutonomyProfile {
    let raw_profile = parse_env_string("LOONGCLAW_AUTONOMY_PROFILE");
    let Some(raw_profile) = raw_profile else {
        return AutonomyProfile::default();
    };

    let parsed_profile = crate::config::parse_autonomy_profile(raw_profile.as_str());
    let Some(profile) = parsed_profile else {
        let default_profile = AutonomyProfile::default();
        let default_profile_id = default_profile.as_str();
        let valid_values = crate::config::AUTONOMY_PROFILE_VALID_VALUES;

        #[allow(clippy::print_stderr)]
        {
            eprintln!(
                "warning: invalid LOONGCLAW_AUTONOMY_PROFILE `{raw_profile}`; falling back to `{default_profile_id}`. supported values: {valid_values}"
            );
        }
        return default_profile;
    };

    profile
}

#[cfg(feature = "feishu-integration")]
fn has_enabled_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    if !config.enabled {
        return false;
    }

    has_secret_binding(config.app_id.as_ref(), config.app_id_env.as_deref())
        && has_secret_binding(config.app_secret.as_ref(), config.app_secret_env.as_deref())
        || config
            .accounts
            .values()
            .any(account_has_enabled_feishu_runtime_credentials)
}

#[cfg(feature = "feishu-integration")]
fn has_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    has_secret_binding(config.app_id.as_ref(), config.app_id_env.as_deref())
        && has_secret_binding(config.app_secret.as_ref(), config.app_secret_env.as_deref())
        || config
            .accounts
            .values()
            .any(account_has_feishu_runtime_credentials)
}

#[cfg(feature = "feishu-integration")]
fn account_has_enabled_feishu_runtime_credentials(
    account: &crate::config::FeishuAccountConfig,
) -> bool {
    account.enabled.unwrap_or(true) && account_has_feishu_runtime_credentials(account)
}

#[cfg(feature = "feishu-integration")]
fn account_has_feishu_runtime_credentials(account: &crate::config::FeishuAccountConfig) -> bool {
    has_secret_binding(account.app_id.as_ref(), account.app_id_env.as_deref())
        && has_secret_binding(
            account.app_secret.as_ref(),
            account.app_secret_env.as_deref(),
        )
}

#[cfg(feature = "feishu-integration")]
fn has_secret_binding(secret_ref: Option<&SecretRef>, env_name: Option<&str>) -> bool {
    if let Some(secret_ref) = secret_ref {
        let explicit_env_name = secret_ref.explicit_env_name();
        if let Some(explicit_env_name) = explicit_env_name {
            let resolved_env_value = std::env::var(explicit_env_name.as_str()).ok();
            let has_resolved_env_value = resolved_env_value
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            return has_resolved_env_value;
        }

        if has_configured_secret_ref(Some(secret_ref)) {
            return true;
        }
    }

    env_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|name| std::env::var(name).ok())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

static TOOL_RUNTIME_CONFIG: OnceLock<ToolRuntimeConfig> = OnceLock::new();

/// Initialise the process-wide tool runtime config.
///
/// Returns `Ok(())` on the first call.  Subsequent calls return
/// `Err` because the `OnceLock` rejects duplicate initialisation.
pub fn init_tool_runtime_config(config: ToolRuntimeConfig) -> Result<(), String> {
    TOOL_RUNTIME_CONFIG.set(config).map_err(|_err| {
        "tool runtime config already initialised (duplicate init_tool_runtime_config call)"
            .to_owned()
    })
}

/// Return the process-wide tool runtime config.
///
/// If `init_tool_runtime_config` was never called the config is lazily
/// populated from environment variables (backward-compat path).
pub fn get_tool_runtime_config() -> &'static ToolRuntimeConfig {
    TOOL_RUNTIME_CONFIG.get_or_init(ToolRuntimeConfig::from_env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    #[cfg(feature = "feishu-integration")]
    use std::collections::BTreeMap;

    fn clear_tool_runtime_env(env: &mut ScopedEnv) {
        for key in [
            "LOONGCLAW_CONFIG_PATH",
            "LOONGCLAW_FILE_ROOT",
            "LOONGCLAW_TOOL_SESSIONS_ENABLED",
            "LOONGCLAW_TOOL_SESSIONS_ALLOW_MUTATION",
            "LOONGCLAW_TOOL_MESSAGES_ENABLED",
            "LOONGCLAW_TOOL_DELEGATE_ENABLED",
            "LOONGCLAW_RUNTIME_SELF_MAX_SOURCE_CHARS",
            "LOONGCLAW_RUNTIME_SELF_MAX_TOTAL_CHARS",
            "LOONGCLAW_BROWSER_ENABLED",
            "LOONGCLAW_BROWSER_MAX_SESSIONS",
            "LOONGCLAW_BROWSER_MAX_LINKS",
            "LOONGCLAW_BROWSER_MAX_TEXT_CHARS",
            "LOONGCLAW_BROWSER_COMPANION_ENABLED",
            "LOONGCLAW_BROWSER_COMPANION_READY",
            "LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS",
            "LOONGCLAW_BROWSER_COMPANION_COMMAND",
            "LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION",
            "LOONGCLAW_WEB_FETCH_ENABLED",
            "LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
            "LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS",
            "LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS",
            "LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS",
            "LOONGCLAW_WEB_FETCH_MAX_BYTES",
            "LOONGCLAW_WEB_FETCH_MAX_REDIRECTS",
            "LOONGCLAW_WEB_SEARCH_ENABLED",
            "LOONGCLAW_WEB_SEARCH_PROVIDER",
            "LOONGCLAW_WEB_SEARCH_TIMEOUT_SECONDS",
            "LOONGCLAW_WEB_SEARCH_MAX_RESULTS",
            "LOONGCLAW_AUTONOMY_PROFILE",
            "BRAVE_API_KEY",
            "TAVILY_API_KEY",
            "PERPLEXITY_API_KEY",
            "EXA_API_KEY",
            "JINA_API_KEY",
            "JINA_AUTH_TOKEN",
            "LOONGCLAW_EXTERNAL_SKILLS_ENABLED",
            "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
            "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
            "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
            "LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT",
            "LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
        ] {
            env.remove(key);
        }
    }

    #[cfg(feature = "feishu-integration")]
    fn clear_feishu_runtime_env(env: &mut ScopedEnv) {
        env.remove("FEISHU_APP_ID");
        env.remove("FEISHU_APP_SECRET");
    }

    #[test]
    fn tool_runtime_config_from_env_defaults() {
        let config = ToolRuntimeConfig::default();
        assert!(config.file_root.is_none());
        assert!(config.config_path.is_none());
        assert!(config.sessions_enabled);
        assert!(!config.sessions_allow_mutation);
        assert!(!config.messages_enabled);
        assert!(config.delegate_enabled);
        assert_eq!(
            config.runtime_self.max_source_chars,
            crate::config::DEFAULT_RUNTIME_SELF_MAX_SOURCE_CHARS
        );
        assert_eq!(
            config.runtime_self.max_total_chars,
            crate::config::DEFAULT_RUNTIME_SELF_MAX_TOTAL_CHARS
        );
        assert!(config.browser.enabled);
        assert_eq!(config.browser.max_sessions, 8);
        assert_eq!(config.browser.max_links, 40);
        assert_eq!(config.browser.max_text_chars, 6000);
        assert!(!config.browser_companion.enabled);
        assert!(!config.browser_companion.ready);
        assert!(config.browser_companion.command.is_none());
        assert!(config.browser_companion.expected_version.is_none());
        assert!(config.web_fetch.enabled);
        assert!(!config.web_fetch.allow_private_hosts);
        assert!(config.web_fetch.allowed_domains.is_empty());
        assert!(config.web_fetch.blocked_domains.is_empty());
        assert_eq!(config.web_fetch.timeout_seconds, 15);
        assert_eq!(config.web_fetch.max_bytes, 1_048_576);
        assert_eq!(config.web_fetch.max_redirects, 3);
        assert!(config.web_search.enabled);
        assert_eq!(
            config.web_search.default_provider,
            crate::config::DEFAULT_WEB_SEARCH_PROVIDER
        );
        assert!(config.web_search.brave_api_key.is_none());
        assert!(config.web_search.tavily_api_key.is_none());
        assert!(config.web_search.perplexity_api_key.is_none());
        assert!(config.web_search.exa_api_key.is_none());
        assert!(config.web_search.jina_api_key.is_none());
        assert_eq!(
            config.web_search.timeout_seconds,
            crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
        );
        assert_eq!(
            config.web_search.max_results,
            crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS
        );
        assert!(!config.external_skills.enabled);
        assert!(config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.is_empty());
        assert!(config.external_skills.blocked_domains.is_empty());
        assert!(config.external_skills.install_root.is_none());
        assert!(!config.external_skills.auto_expose_installed);
    }

    #[test]
    fn autonomy_profile_runtime_config_defaults_to_discovery_only() {
        let config = ToolRuntimeConfig::default();
        let snapshot = config.autonomy_policy_snapshot();

        assert_eq!(config.autonomy_profile, AutonomyProfile::DiscoveryOnly);
        assert_eq!(snapshot.profile, AutonomyProfile::DiscoveryOnly);
        assert_eq!(
            snapshot.capability_acquisition_mode,
            AutonomyOperationMode::Deny
        );
        assert_eq!(snapshot.provider_switch_mode, AutonomyOperationMode::Deny);
        assert_eq!(snapshot.topology_mutation_mode, AutonomyOperationMode::Deny);
        assert!(!snapshot.requires_kernel_binding);
        assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 0);
        assert_eq!(snapshot.budget.max_provider_switches_per_turn, 0);
        assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 0);
        assert_eq!(AutonomyProfile::DiscoveryOnly.as_str(), "discovery_only");
        assert_eq!(
            AutonomyProfile::GuidedAcquisition.as_str(),
            "guided_acquisition"
        );
        assert_eq!(
            AutonomyProfile::BoundedAutonomous.as_str(),
            "bounded_autonomous"
        );
    }

    #[test]
    fn autonomy_profile_runtime_config_from_loongclaw_config_uses_explicit_profile() {
        let mut config = crate::config::LoongClawConfig::default();
        config.tools.autonomy_profile = AutonomyProfile::GuidedAcquisition;

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);
        let snapshot = runtime.autonomy_policy_snapshot();

        assert_eq!(runtime.autonomy_profile, AutonomyProfile::GuidedAcquisition);
        assert_eq!(
            snapshot.capability_acquisition_mode,
            AutonomyOperationMode::ApprovalRequired
        );
        assert_eq!(
            snapshot.provider_switch_mode,
            AutonomyOperationMode::ApprovalRequired
        );
        assert_eq!(snapshot.topology_mutation_mode, AutonomyOperationMode::Deny);
        assert!(snapshot.requires_kernel_binding);
        assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 1);
        assert_eq!(snapshot.budget.max_provider_switches_per_turn, 1);
        assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 0);
    }

    #[test]
    fn autonomy_profile_runtime_config_compiles_bounded_autonomous_snapshot() {
        let config = ToolRuntimeConfig {
            autonomy_profile: AutonomyProfile::BoundedAutonomous,
            ..ToolRuntimeConfig::default()
        };

        let snapshot = config.autonomy_policy_snapshot();

        assert_eq!(snapshot.profile, AutonomyProfile::BoundedAutonomous);
        assert_eq!(
            snapshot.capability_acquisition_mode,
            AutonomyOperationMode::Allow
        );
        assert_eq!(
            snapshot.provider_switch_mode,
            AutonomyOperationMode::ApprovalRequired
        );
        assert_eq!(snapshot.topology_mutation_mode, AutonomyOperationMode::Deny);
        assert!(snapshot.requires_kernel_binding);
        assert_eq!(snapshot.budget.max_capability_acquisitions_per_turn, 2);
        assert_eq!(snapshot.budget.max_provider_switches_per_turn, 1);
        assert_eq!(snapshot.budget.max_topology_mutations_per_turn, 0);
    }

    #[test]
    fn autonomy_profile_runtime_config_from_env_invalid_value_fails_closed() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        env.set("LOONGCLAW_AUTONOMY_PROFILE", "chaos");

        let runtime = ToolRuntimeConfig::from_env();
        let snapshot = runtime.autonomy_policy_snapshot();

        assert_eq!(runtime.autonomy_profile, AutonomyProfile::DiscoveryOnly);
        assert_eq!(snapshot.profile, AutonomyProfile::DiscoveryOnly);
    }

    #[test]
    fn autonomy_profile_runtime_config_from_env_uses_valid_value() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        env.set("LOONGCLAW_AUTONOMY_PROFILE", "guided_acquisition");

        let runtime = ToolRuntimeConfig::from_env();
        let snapshot = runtime.autonomy_policy_snapshot();

        assert_eq!(runtime.autonomy_profile, AutonomyProfile::GuidedAcquisition);
        assert_eq!(snapshot.profile, AutonomyProfile::GuidedAcquisition);
    }

    /// Deny starts empty so users are not forced to carry
    /// any hardcoded restriction they did not opt into.
    #[test]
    fn default_deny_is_empty() {
        let config = ToolRuntimeConfig::default();
        assert!(config.shell_deny.is_empty());
    }

    /// Explicit config injection overrides defaults — verifies that
    /// non-default values survive construction without env-var leakage.
    #[test]
    fn explicit_config_injection_overrides_defaults() {
        let config = ToolRuntimeConfig {
            sessions_enabled: false,
            sessions_allow_mutation: true,
            messages_enabled: true,
            delegate_enabled: false,
            shell_allow: BTreeSet::from(["git".to_owned(), "cargo".to_owned()]),
            file_root: Some(PathBuf::from("/tmp/test-root")),
            config_path: Some(PathBuf::from("/tmp/test-root/loongclaw.toml")),
            runtime_self: RuntimeSelfRuntimePolicy::from_limits(4_096, 32_768),
            browser: BrowserRuntimePolicy {
                enabled: false,
                max_sessions: 4,
                max_links: 12,
                max_text_chars: 2_048,
            },
            browser_companion: BrowserCompanionRuntimePolicy {
                enabled: true,
                ready: true,
                command: Some("loongclaw-browser-companion".to_owned()),
                expected_version: Some("1.2.3".to_owned()),
                timeout_seconds: 9,
                allow_private_hosts: false,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: false,
                allow_private_hosts: true,
                enforce_allowed_domains: true,
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
                timeout_seconds: 9,
                max_bytes: 262_144,
                max_redirects: 1,
            },
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: false,
                allowed_domains: BTreeSet::from(["skills.sh".to_owned()]),
                blocked_domains: BTreeSet::new(),
                install_root: Some(PathBuf::from("/tmp/test-root/skills")),
                auto_expose_installed: false,
            },
            ..ToolRuntimeConfig::default()
        };
        assert!(config.shell_allow.contains("git"));
        assert!(config.shell_allow.contains("cargo"));
        assert!(!config.shell_allow.contains("echo"));
        assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
        assert_eq!(
            config.config_path,
            Some(PathBuf::from("/tmp/test-root/loongclaw.toml"))
        );
        assert!(!config.sessions_enabled);
        assert!(config.sessions_allow_mutation);
        assert!(config.messages_enabled);
        assert!(!config.delegate_enabled);
        assert_eq!(config.runtime_self.max_source_chars, 4_096);
        assert_eq!(config.runtime_self.max_total_chars, 32_768);
        assert!(!config.browser.enabled);
        assert_eq!(config.browser.max_sessions, 4);
        assert_eq!(config.browser.max_links, 12);
        assert_eq!(config.browser.max_text_chars, 2_048);
        assert!(config.browser_companion.enabled);
        assert!(config.browser_companion.ready);
        assert_eq!(
            config.browser_companion.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(
            config.browser_companion.expected_version.as_deref(),
            Some("1.2.3")
        );
        assert!(!config.web_fetch.enabled);
        assert!(config.web_fetch.allow_private_hosts);
        assert!(
            config
                .web_fetch
                .allowed_domains
                .contains("docs.example.com")
        );
        assert!(
            config
                .web_fetch
                .blocked_domains
                .contains("internal.example")
        );
        assert_eq!(config.web_fetch.timeout_seconds, 9);
        assert_eq!(config.web_fetch.max_bytes, 262_144);
        assert_eq!(config.web_fetch.max_redirects, 1);
        assert!(config.external_skills.enabled);
        assert!(!config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.contains("skills.sh"));
        assert_eq!(
            config.external_skills.install_root,
            Some(PathBuf::from("/tmp/test-root/skills"))
        );
        assert!(!config.external_skills.auto_expose_installed);
    }

    #[test]
    fn file_root_uses_injected_config() {
        let config = ToolRuntimeConfig {
            file_root: Some(PathBuf::from("/tmp/test-root")),
            ..ToolRuntimeConfig::default()
        };
        assert_eq!(config.file_root, Some(PathBuf::from("/tmp/test-root")));
    }

    #[test]
    fn from_env_defaults_to_empty_allowlist() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);

        let config = ToolRuntimeConfig::from_env();
        assert!(config.shell_allow.is_empty());
    }

    #[test]
    fn from_loongclaw_config_projects_browser_companion_policy() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "true");
        let mut config = crate::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.timeout_seconds = 7;
        config.tools.browser_companion.command = Some("loongclaw-browser-companion".to_owned());
        config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());
        config.tools.browser_companion.allow_private_hosts = true;
        config.tools.browser_companion.allowed_domains =
            vec!["Docs.Example.com".to_owned(), "api.example.com".to_owned()];
        config.tools.browser_companion.blocked_domains = vec![
            "internal.example".to_owned(),
            " INTERNAL.EXAMPLE ".to_owned(),
        ];

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);
        assert!(runtime.browser_companion.enabled);
        assert!(runtime.browser_companion.ready);
        assert_eq!(
            runtime.browser_companion.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(
            runtime.browser_companion.expected_version.as_deref(),
            Some("1.2.3")
        );
        assert_eq!(runtime.browser_companion.timeout_seconds, 7);
        assert!(runtime.browser_companion.allow_private_hosts);
        assert!(runtime.browser_companion.enforce_allowed_domains);
        assert_eq!(
            runtime.browser_companion.allowed_domains,
            BTreeSet::from(["api.example.com".to_owned(), "docs.example.com".to_owned()])
        );
        assert_eq!(
            runtime.browser_companion.blocked_domains,
            BTreeSet::from(["internal.example".to_owned()])
        );
    }

    #[test]
    fn from_loongclaw_config_projects_runtime_self_policy() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);

        let mut config = crate::config::LoongClawConfig::default();
        config.tools.runtime_self.max_source_chars = 12_345;
        config.tools.runtime_self.max_total_chars = 67_890;

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);

        assert_eq!(runtime.runtime_self.max_source_chars, 12_345);
        assert_eq!(runtime.runtime_self.max_total_chars, 67_890);
    }

    #[test]
    fn from_loongclaw_config_projects_session_mutation_toggle() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);

        let mut config = crate::config::LoongClawConfig::default();
        config.tools.sessions.allow_mutation = true;

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);

        assert!(runtime.sessions_enabled);
        assert!(runtime.sessions_allow_mutation);
    }

    #[test]
    fn from_loongclaw_config_canonicalizes_web_search_provider_alias() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);

        let mut config = crate::config::LoongClawConfig::default();
        config.tools.web_search.default_provider = "ddg".to_owned();

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);

        assert_eq!(
            runtime.web_search.default_provider,
            crate::config::DEFAULT_WEB_SEARCH_PROVIDER
        );
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn injected_config_overrides_global() {
        let _env = ScopedEnv::new();
        let config = ToolRuntimeConfig {
            file_root: Some(PathBuf::from("/tmp/injected-root")),
            shell_allow: BTreeSet::from(["echo".to_owned()]),
            config_path: Some(PathBuf::from("/tmp/injected-root/loongclaw.toml")),
            ..ToolRuntimeConfig::default()
        };
        let result = crate::tools::execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "shell.exec".to_owned(),
                payload: serde_json::json!({"command": "echo", "args": ["injected"]}),
            },
            &config,
        );
        let outcome = result.expect("echo should be allowed with injected config");
        assert_eq!(outcome.status, "ok");
        assert!(
            outcome.payload["stdout"]
                .as_str()
                .unwrap()
                .contains("injected")
        );
    }

    #[test]
    fn from_env_parses_external_skills_policy() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);
        env.set("LOONGCLAW_TOOL_SESSIONS_ENABLED", "false");
        env.set("LOONGCLAW_TOOL_SESSIONS_ALLOW_MUTATION", "true");
        env.set("LOONGCLAW_TOOL_MESSAGES_ENABLED", "true");
        env.set("LOONGCLAW_TOOL_DELEGATE_ENABLED", "false");
        env.set("LOONGCLAW_BROWSER_ENABLED", "false");
        env.set("LOONGCLAW_BROWSER_MAX_SESSIONS", "4");
        env.set("LOONGCLAW_BROWSER_MAX_LINKS", "12");
        env.set("LOONGCLAW_BROWSER_MAX_TEXT_CHARS", "2048");
        env.set("LOONGCLAW_BROWSER_COMPANION_ENABLED", "true");
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "true");
        env.set("LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS", "11");
        env.set(
            "LOONGCLAW_BROWSER_COMPANION_COMMAND",
            "loongclaw-browser-companion",
        );
        env.set("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION", "1.2.3");
        env.set("LOONGCLAW_WEB_FETCH_ENABLED", "false");
        env.set("LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS", "true");
        env.set(
            "LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS",
            "docs.example.com,api.example.com",
        );
        env.set("LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS", "internal.example");
        env.set("LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS", "9");
        env.set("LOONGCLAW_WEB_FETCH_MAX_BYTES", "262144");
        env.set("LOONGCLAW_WEB_FETCH_MAX_REDIRECTS", "1");
        env.set("LOONGCLAW_EXTERNAL_SKILLS_ENABLED", "true");
        env.set(
            "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
            "false",
        );
        env.set(
            "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
            "skills.sh,clawhub.io",
        );
        env.set(
            "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
            "malicious.example",
        );
        env.set(
            "LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT",
            "/tmp/managed-skills",
        );
        env.set("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED", "false");

        let config = ToolRuntimeConfig::from_env();
        assert!(!config.sessions_enabled);
        assert!(config.sessions_allow_mutation);
        assert!(config.messages_enabled);
        assert!(!config.delegate_enabled);
        assert!(!config.browser.enabled);
        assert_eq!(config.browser.max_sessions, 4);
        assert_eq!(config.browser.max_links, 12);
        assert_eq!(config.browser.max_text_chars, 2_048);
        assert!(config.browser_companion.enabled);
        assert!(config.browser_companion.ready);
        assert_eq!(
            config.browser_companion.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(
            config.browser_companion.expected_version.as_deref(),
            Some("1.2.3")
        );
        assert_eq!(config.browser_companion.timeout_seconds, 11);
        assert!(!config.web_fetch.enabled);
        assert!(config.web_fetch.allow_private_hosts);
        assert!(
            config
                .web_fetch
                .allowed_domains
                .contains("docs.example.com")
        );
        assert!(config.web_fetch.allowed_domains.contains("api.example.com"));
        assert!(
            config
                .web_fetch
                .blocked_domains
                .contains("internal.example")
        );
        assert_eq!(config.web_fetch.timeout_seconds, 9);
        assert_eq!(config.web_fetch.max_bytes, 262_144);
        assert_eq!(config.web_fetch.max_redirects, 1);
        assert!(config.web_search.enabled);
        assert_eq!(
            config.web_search.default_provider,
            crate::config::DEFAULT_WEB_SEARCH_PROVIDER
        );
        assert!(config.web_search.brave_api_key.is_none());
        assert!(config.web_search.tavily_api_key.is_none());
        assert_eq!(
            config.web_search.timeout_seconds,
            crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS
        );
        assert_eq!(
            config.web_search.max_results,
            crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS
        );
        assert!(config.external_skills.enabled);
        assert!(!config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.contains("skills.sh"));
        assert!(
            config
                .external_skills
                .allowed_domains
                .contains("clawhub.io")
        );
        assert!(
            config
                .external_skills
                .blocked_domains
                .contains("malicious.example")
        );
        assert_eq!(
            config.external_skills.install_root,
            Some(PathBuf::from("/tmp/managed-skills"))
        );
        assert!(!config.external_skills.auto_expose_installed);
    }

    #[test]
    fn from_env_clamps_runtime_self_policy() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);
        env.set("LOONGCLAW_RUNTIME_SELF_MAX_SOURCE_CHARS", "999999");
        env.set("LOONGCLAW_RUNTIME_SELF_MAX_TOTAL_CHARS", "1");

        let config = ToolRuntimeConfig::from_env();

        assert_eq!(
            config.runtime_self.max_source_chars,
            crate::config::MAX_RUNTIME_SELF_MAX_SOURCE_CHARS
        );
        assert_eq!(
            config.runtime_self.max_total_chars,
            crate::config::MIN_RUNTIME_SELF_MAX_TOTAL_CHARS
        );
    }

    #[test]
    fn from_env_canonicalizes_and_clamps_web_search_policy() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);
        env.set("LOONGCLAW_WEB_SEARCH_PROVIDER", "DDG");
        env.set("LOONGCLAW_WEB_SEARCH_TIMEOUT_SECONDS", "999");
        env.set("LOONGCLAW_WEB_SEARCH_MAX_RESULTS", "42");
        env.set(
            crate::config::WEB_SEARCH_BRAVE_API_KEY_ENV,
            "brave-test-key",
        );
        env.set(
            crate::config::WEB_SEARCH_TAVILY_API_KEY_ENV,
            "tavily-test-key",
        );
        env.set(
            crate::config::WEB_SEARCH_PERPLEXITY_API_KEY_ENV,
            "perplexity-test-key",
        );
        env.set(crate::config::WEB_SEARCH_EXA_API_KEY_ENV, "exa-test-key");
        env.set(
            crate::config::WEB_SEARCH_JINA_AUTH_TOKEN_ENV,
            "jina-test-key",
        );

        let config = ToolRuntimeConfig::from_env();

        assert_eq!(
            config.web_search.default_provider,
            crate::config::DEFAULT_WEB_SEARCH_PROVIDER
        );
        assert_eq!(config.web_search.timeout_seconds, 60);
        assert_eq!(config.web_search.max_results, 10);
        assert_eq!(
            config.web_search.brave_api_key.as_deref(),
            Some("brave-test-key")
        );
        assert_eq!(
            config.web_search.tavily_api_key.as_deref(),
            Some("tavily-test-key")
        );
        assert_eq!(
            config.web_search.perplexity_api_key.as_deref(),
            Some("perplexity-test-key")
        );
        assert_eq!(
            config.web_search.exa_api_key.as_deref(),
            Some("exa-test-key")
        );
        assert_eq!(
            config.web_search.jina_api_key.as_deref(),
            Some("jina-test-key")
        );
    }

    #[test]
    fn from_loongclaw_config_resolves_inline_env_refs_for_web_search_credentials() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        #[cfg(feature = "feishu-integration")]
        clear_feishu_runtime_env(&mut env);
        env.set("TEAM_EXA_KEY", "exa-inline-env");

        let mut config = LoongClawConfig::default();
        config.tools.web_search.default_provider =
            crate::config::WEB_SEARCH_PROVIDER_EXA.to_owned();
        config.tools.web_search.exa_api_key = Some("${TEAM_EXA_KEY}".to_owned());

        let runtime = ToolRuntimeConfig::from_loongclaw_config(&config, None);

        assert_eq!(
            runtime.web_search.default_provider,
            crate::config::WEB_SEARCH_PROVIDER_EXA
        );
        assert_eq!(
            runtime.web_search.exa_api_key.as_deref(),
            Some("exa-inline-env")
        );
    }

    #[test]
    fn external_skills_policy_struct_construction() {
        let policy = ExternalSkillsRuntimePolicy {
            enabled: true,
            require_download_approval: false,
            allowed_domains: BTreeSet::from(["skills.sh".to_owned(), "clawhub.io".to_owned()]),
            blocked_domains: BTreeSet::from(["malicious.example".to_owned()]),
            install_root: Some(PathBuf::from("/tmp/managed-skills")),
            auto_expose_installed: false,
        };

        assert!(policy.enabled);
        assert!(!policy.require_download_approval);
        assert!(policy.allowed_domains.contains("skills.sh"));
        assert!(policy.allowed_domains.contains("clawhub.io"));
        assert!(policy.blocked_domains.contains("malicious.example"));
        assert_eq!(
            policy.install_root,
            Some(PathBuf::from("/tmp/managed-skills"))
        );
        assert!(!policy.auto_expose_installed);
    }

    #[test]
    fn browser_policy_struct_construction() {
        let policy = BrowserRuntimePolicy {
            enabled: false,
            max_sessions: 4,
            max_links: 12,
            max_text_chars: 2_048,
        };

        assert!(!policy.enabled);
        assert_eq!(policy.max_sessions, 4);
        assert_eq!(policy.max_links, 12);
        assert_eq!(policy.max_text_chars, 2_048);
    }

    #[test]
    fn browser_companion_policy_struct_construction() {
        let policy = BrowserCompanionRuntimePolicy {
            enabled: true,
            ready: false,
            command: Some("loongclaw-browser-companion".to_owned()),
            expected_version: Some("1.2.3".to_owned()),
            timeout_seconds: 9,
            allow_private_hosts: false,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
        };

        assert!(policy.enabled);
        assert!(!policy.ready);
        assert!(!policy.is_runtime_ready());
        assert_eq!(
            policy.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(policy.expected_version.as_deref(), Some("1.2.3"));
        assert_eq!(policy.timeout_seconds, 9);
        assert!(!policy.allow_private_hosts);
        assert!(policy.enforce_allowed_domains);
        assert_eq!(
            policy.allowed_domains,
            BTreeSet::from(["docs.example.com".to_owned()])
        );
        assert_eq!(
            policy.blocked_domains,
            BTreeSet::from(["internal.example".to_owned()])
        );
    }

    #[test]
    fn browser_companion_policy_from_tool_config_clamps_zero_timeout() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        let mut config = crate::config::ToolConfig::default();
        config.browser_companion.enabled = true;
        config.browser_companion.timeout_seconds = 0;
        config.browser_companion.allowed_domains = vec!["Docs.Example.com".to_owned()];
        config.browser_companion.blocked_domains = vec!["internal.example".to_owned()];

        let policy = browser_companion_runtime_policy_from_tool_config(&config);

        assert_eq!(policy.timeout_seconds, 1);
        assert!(policy.enforce_allowed_domains);
        assert_eq!(
            policy.allowed_domains,
            BTreeSet::from(["docs.example.com".to_owned()])
        );
        assert_eq!(
            policy.blocked_domains,
            BTreeSet::from(["internal.example".to_owned()])
        );
    }

    #[test]
    fn browser_companion_policy_with_env_fallback_uses_runtime_exports_for_default_config() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        env.set("LOONGCLAW_BROWSER_COMPANION_ENABLED", "true");
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "false");
        env.set(
            "LOONGCLAW_BROWSER_COMPANION_COMMAND",
            "loongclaw-browser-companion",
        );
        env.set("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION", "1.2.3");
        env.set("LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS", "11");

        let policy = browser_companion_runtime_policy_with_env_fallback(
            &crate::config::ToolConfig::default(),
        );

        assert!(policy.enabled);
        assert!(!policy.ready);
        assert_eq!(
            policy.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(policy.expected_version.as_deref(), Some("1.2.3"));
        assert_eq!(policy.timeout_seconds, 11);
    }

    #[test]
    fn web_fetch_policy_struct_construction() {
        let policy = WebFetchRuntimePolicy {
            enabled: false,
            allow_private_hosts: true,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
            timeout_seconds: 9,
            max_bytes: 262_144,
            max_redirects: 1,
        };

        assert!(!policy.enabled);
        assert!(policy.allow_private_hosts);
        assert!(policy.enforce_allowed_domains);
        assert!(policy.allowed_domains.contains("docs.example.com"));
        assert!(policy.blocked_domains.contains("internal.example"));
        assert_eq!(policy.timeout_seconds, 9);
        assert_eq!(policy.max_bytes, 262_144);
        assert_eq!(policy.max_redirects, 1);
    }

    #[test]
    fn tool_runtime_config_narrowed_intersects_web_domains_and_clamps_browser_limits() {
        let base = ToolRuntimeConfig {
            browser: BrowserRuntimePolicy {
                enabled: true,
                max_sessions: 4,
                max_links: 12,
                max_text_chars: 2_048,
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: true,
                enforce_allowed_domains: true,
                allowed_domains: BTreeSet::from([
                    "docs.example.com".to_owned(),
                    "api.example.com".to_owned(),
                ]),
                blocked_domains: BTreeSet::from(["blocked.example.com".to_owned()]),
                timeout_seconds: 15,
                max_bytes: 8_192,
                max_redirects: 4,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(1),
                max_links: Some(6),
                max_text_chars: Some(512),
            },
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(false),
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
                timeout_seconds: Some(5),
                max_bytes: Some(4_096),
                max_redirects: Some(2),
            },
        };

        let effective = base.narrowed(&narrowing);

        assert_eq!(effective.browser.max_sessions, 1);
        assert_eq!(effective.browser.max_links, 6);
        assert_eq!(effective.browser.max_text_chars, 512);
        assert!(!effective.web_fetch.allow_private_hosts);
        assert_eq!(
            effective.web_fetch.allowed_domains,
            BTreeSet::from(["docs.example.com".to_owned()])
        );
        assert!(effective.web_fetch.enforce_allowed_domains);
        assert_eq!(
            effective.web_fetch.blocked_domains,
            BTreeSet::from([
                "blocked.example.com".to_owned(),
                "deny.example.com".to_owned(),
            ])
        );
        assert_eq!(effective.web_fetch.timeout_seconds, 5);
        assert_eq!(effective.web_fetch.max_bytes, 4_096);
        assert_eq!(effective.web_fetch.max_redirects, 2);
    }

    #[test]
    fn tool_runtime_config_narrowed_uses_child_allowlist_when_parent_has_none() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: false,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: 15,
                max_bytes: 8_192,
                max_redirects: 4,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: None,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: None,
                max_bytes: None,
                max_redirects: None,
            },
            ..ToolRuntimeNarrowing::default()
        };

        let effective = base.narrowed(&narrowing);

        assert_eq!(
            effective.web_fetch.allowed_domains,
            BTreeSet::from(["docs.example.com".to_owned()])
        );
        assert!(effective.web_fetch.enforce_allowed_domains);
    }

    #[test]
    fn tool_runtime_config_narrowed_fail_closes_disjoint_allowlists() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: false,
                enforce_allowed_domains: true,
                allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: 15,
                max_bytes: 8_192,
                max_redirects: 4,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                ..WebFetchRuntimeNarrowing::default()
            },
            ..ToolRuntimeNarrowing::default()
        };

        let effective = base.narrowed(&narrowing);

        assert!(effective.web_fetch.enforce_allowed_domains);
        assert!(
            effective.web_fetch.allowed_domains.is_empty(),
            "disjoint allowlists should preserve an enforced empty intersection"
        );
    }

    #[test]
    fn tool_runtime_config_narrowed_preserves_existing_deny_all_allowlist() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: false,
                enforce_allowed_domains: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: 15,
                max_bytes: 8_192,
                max_redirects: 4,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                ..WebFetchRuntimeNarrowing::default()
            },
            ..ToolRuntimeNarrowing::default()
        };

        let effective = base.narrowed(&narrowing);

        assert!(effective.web_fetch.enforce_allowed_domains);
        assert!(
            effective.web_fetch.allowed_domains.is_empty(),
            "an existing fail-closed allowlist should not be widened by later narrowing"
        );
    }

    #[test]
    fn tool_runtime_narrowing_intersect_fail_closes_disjoint_allowlists() {
        let left = ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(1),
                ..BrowserRuntimeNarrowing::default()
            },
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(false),
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["deny-left.example.com".to_owned()]),
                timeout_seconds: Some(5),
                max_bytes: Some(4_096),
                max_redirects: Some(2),
            },
        };
        let right = ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(3),
                ..BrowserRuntimeNarrowing::default()
            },
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: None,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["deny-right.example.com".to_owned()]),
                timeout_seconds: Some(9),
                max_bytes: Some(8_192),
                max_redirects: Some(4),
            },
        };

        let effective = left.intersect(&right);

        assert_eq!(effective.browser.max_sessions, Some(1));
        assert_eq!(effective.web_fetch.allow_private_hosts, Some(false));
        assert!(effective.web_fetch.enforce_allowed_domains);
        assert!(
            effective.web_fetch.allowed_domains.is_empty(),
            "disjoint allowlists should collapse to an enforced empty intersection"
        );
        assert_eq!(
            effective.web_fetch.blocked_domains,
            BTreeSet::from([
                "deny-left.example.com".to_owned(),
                "deny-right.example.com".to_owned(),
            ])
        );
        assert_eq!(effective.web_fetch.timeout_seconds, Some(5));
        assert_eq!(effective.web_fetch.max_bytes, Some(4_096));
        assert_eq!(effective.web_fetch.max_redirects, Some(2));
    }

    #[test]
    fn delegate_child_prompt_summary_returns_none_when_narrowing_is_empty() {
        assert_eq!(
            ToolRuntimeConfig::default()
                .delegate_child_prompt_summary(&ToolRuntimeNarrowing::default()),
            None
        );
    }

    #[test]
    fn delegate_child_prompt_summary_is_effective_stable_and_sparse() {
        let base = ToolRuntimeConfig {
            browser: BrowserRuntimePolicy {
                enabled: true,
                max_sessions: 1,
                max_links: 4,
                max_text_chars: 1_024,
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: false,
                enforce_allowed_domains: true,
                allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["base-block.example.com".to_owned()]),
                timeout_seconds: 3,
                max_bytes: 2_048,
                max_redirects: 1,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(8),
                max_links: Some(8),
                max_text_chars: Some(512),
            },
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(true),
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
                timeout_seconds: Some(5),
                max_bytes: Some(4_096),
                max_redirects: Some(2),
            },
        };

        let summary = base
            .delegate_child_prompt_summary(&narrowing)
            .expect("delegate child prompt summary");

        assert_eq!(
            summary,
            "[delegate_child_runtime_contract]\n\
Plan within these child-session runtime limits:\n\
- web.fetch private hosts: denied\n\
- web.fetch allowed domains: none (effective intersection is empty)\n\
- web.fetch blocked domains: base-block.example.com, deny.example.com\n\
- web.fetch timeout seconds: 3\n\
- web.fetch max bytes: 2048\n\
- web.fetch max redirects: 1\n\
- browser max sessions: 1\n\
- browser max links: 4\n\
- browser max text chars: 512\n\
Treat these as enforced limits for this child session."
        );
    }

    #[test]
    fn delegate_child_prompt_summary_omits_disabled_web_fetch() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: false,
                allow_private_hosts: true,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: 30,
                max_bytes: 1_048_576,
                max_redirects: 5,
            },
            browser: BrowserRuntimePolicy {
                enabled: true,
                max_sessions: 4,
                max_links: 16,
                max_text_chars: 8_192,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(false),
                timeout_seconds: Some(5),
                ..WebFetchRuntimeNarrowing::default()
            },
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(2),
                ..BrowserRuntimeNarrowing::default()
            },
        };

        let summary = base
            .delegate_child_prompt_summary(&narrowing)
            .expect("should still render browser section");

        assert!(
            !summary.contains("web.fetch"),
            "disabled web_fetch fields should not appear in prompt summary: {summary}"
        );
        assert!(
            summary.contains("- browser max sessions: 2"),
            "enabled browser fields should still appear: {summary}"
        );
    }

    #[test]
    fn delegate_child_prompt_summary_omits_disabled_browser() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: true,
                allow_private_hosts: true,
                enforce_allowed_domains: false,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                timeout_seconds: 30,
                max_bytes: 1_048_576,
                max_redirects: 5,
            },
            browser: BrowserRuntimePolicy {
                enabled: false,
                max_sessions: 4,
                max_links: 16,
                max_text_chars: 8_192,
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                timeout_seconds: Some(5),
                ..WebFetchRuntimeNarrowing::default()
            },
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(2),
                max_links: Some(8),
                ..BrowserRuntimeNarrowing::default()
            },
        };

        let summary = base
            .delegate_child_prompt_summary(&narrowing)
            .expect("should still render web_fetch section");

        assert!(
            !summary.contains("browser"),
            "disabled browser fields should not appear in prompt summary: {summary}"
        );
        assert!(
            summary.contains("- web.fetch timeout seconds: 5"),
            "enabled web_fetch fields should still appear: {summary}"
        );
    }

    #[test]
    fn delegate_child_prompt_summary_returns_none_when_all_tools_disabled() {
        let base = ToolRuntimeConfig {
            web_fetch: WebFetchRuntimePolicy {
                enabled: false,
                ..WebFetchRuntimePolicy::default()
            },
            browser: BrowserRuntimePolicy {
                enabled: false,
                ..BrowserRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        };
        let narrowing = ToolRuntimeNarrowing {
            web_fetch: WebFetchRuntimeNarrowing {
                timeout_seconds: Some(5),
                ..WebFetchRuntimeNarrowing::default()
            },
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(2),
                ..BrowserRuntimeNarrowing::default()
            },
        };

        assert_eq!(
            base.delegate_child_prompt_summary(&narrowing),
            None,
            "should return None when all narrowed tools are disabled"
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn from_env_enables_feishu_runtime_when_credentials_exist() {
        let mut env = ScopedEnv::new();
        clear_tool_runtime_env(&mut env);
        clear_feishu_runtime_env(&mut env);
        env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
        env.set("FEISHU_APP_SECRET", "env-secret");

        let config = ToolRuntimeConfig::from_env();
        let feishu = config
            .feishu
            .as_ref()
            .expect("feishu runtime should be enabled from env");

        assert!(feishu.channel.enabled);
        assert_eq!(feishu.channel.app_id_env.as_deref(), Some("FEISHU_APP_ID"));
        assert_eq!(
            feishu.channel.app_secret_env.as_deref(),
            Some("FEISHU_APP_SECRET")
        );
        assert_eq!(
            feishu.integration.resolved_sqlite_path(),
            crate::config::default_loongclaw_home().join("feishu.sqlite3")
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn from_loongclaw_config_ignores_disabled_feishu_channel_even_when_root_credentials_exist() {
        let config = crate::config::LoongClawConfig {
            feishu: crate::config::FeishuChannelConfig {
                enabled: false,
                app_id: Some(loongclaw_contracts::SecretRef::Inline(
                    "cli_disabled_root".to_owned(),
                )),
                app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                    "disabled-root-secret".to_owned(),
                )),
                ..crate::config::FeishuChannelConfig::default()
            },
            ..crate::config::LoongClawConfig::default()
        };

        assert!(
            FeishuToolRuntimeConfig::from_loongclaw_config(&config).is_none(),
            "disabled Feishu channel should not expose Feishu tools through runtime config"
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn from_loongclaw_config_ignores_disabled_feishu_accounts_when_detecting_runtime() {
        let mut env = ScopedEnv::new();
        env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
        env.set("FEISHU_APP_SECRET", "env-secret");

        let config = crate::config::LoongClawConfig {
            feishu: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id_env: None,
                app_secret_env: None,
                accounts: BTreeMap::from([(
                    "disabled_account".to_owned(),
                    crate::config::FeishuAccountConfig {
                        enabled: Some(false),
                        app_id: Some(loongclaw_contracts::SecretRef::Inline(
                            "cli_disabled_account".to_owned(),
                        )),
                        app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                            "disabled-account-secret".to_owned(),
                        )),
                        ..crate::config::FeishuAccountConfig::default()
                    },
                )]),
                ..crate::config::FeishuChannelConfig::default()
            },
            ..crate::config::LoongClawConfig::default()
        };

        assert!(
            FeishuToolRuntimeConfig::from_loongclaw_config(&config).is_none(),
            "disabled Feishu accounts should not enable Feishu tool runtime on their own"
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn from_loongclaw_config_requires_resolved_env_values_for_typed_feishu_secret_refs() {
        let mut env = ScopedEnv::new();
        clear_feishu_runtime_env(&mut env);

        let config = crate::config::LoongClawConfig {
            feishu: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some(loongclaw_contracts::SecretRef::Env {
                    env: "FEISHU_APP_ID".to_owned(),
                }),
                app_secret: Some(loongclaw_contracts::SecretRef::Env {
                    env: "FEISHU_APP_SECRET".to_owned(),
                }),
                app_id_env: None,
                app_secret_env: None,
                ..crate::config::FeishuChannelConfig::default()
            },
            ..crate::config::LoongClawConfig::default()
        };

        assert!(
            FeishuToolRuntimeConfig::from_loongclaw_config(&config).is_none(),
            "missing env values should not enable Feishu runtime for typed env refs"
        );

        env.set("FEISHU_APP_ID", "cli_env_a1b2c3");
        env.set("FEISHU_APP_SECRET", "env-secret");

        assert!(
            FeishuToolRuntimeConfig::from_loongclaw_config(&config).is_some(),
            "resolved env values should enable Feishu runtime for typed env refs"
        );
    }
}
