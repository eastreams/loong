use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::shell_policy_ext::ShellPolicyDefault;
use crate::config::LoongClawConfig;
#[cfg(feature = "feishu-integration")]
use crate::config::{FeishuChannelConfig, FeishuIntegrationConfig};

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowserCompanionRuntimePolicy {
    pub enabled: bool,
    pub ready: bool,
    pub command: Option<String>,
    pub expected_version: Option<String>,
}

impl BrowserCompanionRuntimePolicy {
    #[must_use]
    pub fn is_runtime_ready(&self) -> bool {
        self.enabled && self.ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchRuntimePolicy {
    pub enabled: bool,
    pub allow_private_hosts: bool,
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
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: crate::config::DEFAULT_WEB_FETCH_TIMEOUT_SECONDS,
            max_bytes: crate::config::DEFAULT_WEB_FETCH_MAX_BYTES,
            max_redirects: crate::config::DEFAULT_WEB_FETCH_MAX_REDIRECTS,
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
    pub messages_enabled: bool,
    pub delegate_enabled: bool,
    pub browser: BrowserRuntimePolicy,
    pub browser_companion: BrowserCompanionRuntimePolicy,
    pub web_fetch: WebFetchRuntimePolicy,
    pub external_skills: ExternalSkillsRuntimePolicy,
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
            messages_enabled: false,
            delegate_enabled: true,
            browser: BrowserRuntimePolicy::default(),
            browser_companion: BrowserCompanionRuntimePolicy::default(),
            web_fetch: WebFetchRuntimePolicy::default(),
            external_skills: ExternalSkillsRuntimePolicy::default(),
            #[cfg(feature = "feishu-integration")]
            feishu: None,
        }
    }
}

impl ToolRuntimeConfig {
    pub fn from_loongclaw_config(config: &LoongClawConfig, config_path: Option<&Path>) -> Self {
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
            messages_enabled: config.tools.messages.enabled,
            delegate_enabled: config.tools.delegate.enabled,
            browser: BrowserRuntimePolicy {
                enabled: config.tools.browser.enabled,
                max_sessions: config.tools.browser.max_sessions,
                max_links: config.tools.browser.max_links,
                max_text_chars: config.tools.browser.max_text_chars,
            },
            browser_companion: BrowserCompanionRuntimePolicy {
                enabled: config.tools.browser_companion.enabled,
                ready: parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false),
                command: normalize_optional_string(
                    config.tools.browser_companion.command.as_deref(),
                ),
                expected_version: normalize_optional_string(
                    config.tools.browser_companion.expected_version.as_deref(),
                ),
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: config.tools.web.enabled,
                allow_private_hosts: config.tools.web.allow_private_hosts,
                allowed_domains: config
                    .tools
                    .web
                    .normalized_allowed_domains()
                    .into_iter()
                    .collect(),
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
        let messages_enabled = parse_env_bool("LOONGCLAW_TOOL_MESSAGES_ENABLED").unwrap_or(false);
        let delegate_enabled = parse_env_bool("LOONGCLAW_TOOL_DELEGATE_ENABLED").unwrap_or(true);
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

        Self {
            file_root,
            config_path,
            sessions_enabled,
            messages_enabled,
            delegate_enabled,
            browser: BrowserRuntimePolicy {
                enabled: browser_enabled,
                max_sessions: browser_max_sessions,
                max_links: browser_max_links,
                max_text_chars: browser_max_text_chars,
            },
            browser_companion: BrowserCompanionRuntimePolicy {
                enabled: browser_companion_enabled,
                ready: browser_companion_ready,
                command: browser_companion_command,
                expected_version: browser_companion_expected_version,
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: web_fetch_enabled,
                allow_private_hosts: web_fetch_allow_private_hosts,
                allowed_domains: web_fetch_allowed_domains,
                blocked_domains: web_fetch_blocked_domains,
                timeout_seconds: web_fetch_timeout_seconds,
                max_bytes: web_fetch_max_bytes,
                max_redirects: web_fetch_max_redirects,
            },
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
}

pub(crate) fn browser_companion_runtime_policy_from_tool_config(
    config: &crate::config::ToolConfig,
) -> BrowserCompanionRuntimePolicy {
    BrowserCompanionRuntimePolicy {
        enabled: config.browser_companion.enabled,
        ready: parse_env_bool("LOONGCLAW_BROWSER_COMPANION_READY").unwrap_or(false),
        command: normalize_optional_string(config.browser_companion.command.as_deref()),
        expected_version: normalize_optional_string(
            config.browser_companion.expected_version.as_deref(),
        ),
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

#[cfg(feature = "feishu-integration")]
fn has_enabled_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    if !config.enabled {
        return false;
    }

    has_secret_binding(config.app_id.as_deref(), config.app_id_env.as_deref())
        && has_secret_binding(
            config.app_secret.as_deref(),
            config.app_secret_env.as_deref(),
        )
        || config
            .accounts
            .values()
            .any(account_has_enabled_feishu_runtime_credentials)
}

#[cfg(feature = "feishu-integration")]
fn has_feishu_runtime_credentials(config: &FeishuChannelConfig) -> bool {
    has_secret_binding(config.app_id.as_deref(), config.app_id_env.as_deref())
        && has_secret_binding(
            config.app_secret.as_deref(),
            config.app_secret_env.as_deref(),
        )
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
    has_secret_binding(account.app_id.as_deref(), account.app_id_env.as_deref())
        && has_secret_binding(
            account.app_secret.as_deref(),
            account.app_secret_env.as_deref(),
        )
}

#[cfg(feature = "feishu-integration")]
fn has_secret_binding(inline: Option<&str>, env_name: Option<&str>) -> bool {
    inline
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        || env_name
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
            "LOONGCLAW_TOOL_MESSAGES_ENABLED",
            "LOONGCLAW_TOOL_DELEGATE_ENABLED",
            "LOONGCLAW_BROWSER_ENABLED",
            "LOONGCLAW_BROWSER_MAX_SESSIONS",
            "LOONGCLAW_BROWSER_MAX_LINKS",
            "LOONGCLAW_BROWSER_MAX_TEXT_CHARS",
            "LOONGCLAW_BROWSER_COMPANION_ENABLED",
            "LOONGCLAW_BROWSER_COMPANION_READY",
            "LOONGCLAW_BROWSER_COMPANION_COMMAND",
            "LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION",
            "LOONGCLAW_WEB_FETCH_ENABLED",
            "LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
            "LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS",
            "LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS",
            "LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS",
            "LOONGCLAW_WEB_FETCH_MAX_BYTES",
            "LOONGCLAW_WEB_FETCH_MAX_REDIRECTS",
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
        assert!(!config.messages_enabled);
        assert!(config.delegate_enabled);
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
        assert!(!config.external_skills.enabled);
        assert!(config.external_skills.require_download_approval);
        assert!(config.external_skills.allowed_domains.is_empty());
        assert!(config.external_skills.blocked_domains.is_empty());
        assert!(config.external_skills.install_root.is_none());
        assert!(!config.external_skills.auto_expose_installed);
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
            messages_enabled: true,
            delegate_enabled: false,
            shell_allow: BTreeSet::from(["git".to_owned(), "cargo".to_owned()]),
            file_root: Some(PathBuf::from("/tmp/test-root")),
            config_path: Some(PathBuf::from("/tmp/test-root/loongclaw.toml")),
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
            },
            web_fetch: WebFetchRuntimePolicy {
                enabled: false,
                allow_private_hosts: true,
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
        config.tools.browser_companion.command = Some("loongclaw-browser-companion".to_owned());
        config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());

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
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn injected_config_overrides_global() {
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
        env.set("LOONGCLAW_TOOL_MESSAGES_ENABLED", "true");
        env.set("LOONGCLAW_TOOL_DELEGATE_ENABLED", "false");
        env.set("LOONGCLAW_BROWSER_ENABLED", "false");
        env.set("LOONGCLAW_BROWSER_MAX_SESSIONS", "4");
        env.set("LOONGCLAW_BROWSER_MAX_LINKS", "12");
        env.set("LOONGCLAW_BROWSER_MAX_TEXT_CHARS", "2048");
        env.set("LOONGCLAW_BROWSER_COMPANION_ENABLED", "true");
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "true");
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
        };

        assert!(policy.enabled);
        assert!(!policy.ready);
        assert!(!policy.is_runtime_ready());
        assert_eq!(
            policy.command.as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(policy.expected_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn web_fetch_policy_struct_construction() {
        let policy = WebFetchRuntimePolicy {
            enabled: false,
            allow_private_hosts: true,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["internal.example".to_owned()]),
            timeout_seconds: 9,
            max_bytes: 262_144,
            max_redirects: 1,
        };

        assert!(!policy.enabled);
        assert!(policy.allow_private_hosts);
        assert!(policy.allowed_domains.contains("docs.example.com"));
        assert!(policy.blocked_domains.contains("internal.example"));
        assert_eq!(policy.timeout_seconds, 9);
        assert_eq!(policy.max_bytes, 262_144);
        assert_eq!(policy.max_redirects, 1);
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
                app_id: Some("cli_disabled_root".to_owned()),
                app_secret: Some("disabled-root-secret".to_owned()),
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
                        app_id: Some("cli_disabled_account".to_owned()),
                        app_secret: Some("disabled-account-secret".to_owned()),
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
}
