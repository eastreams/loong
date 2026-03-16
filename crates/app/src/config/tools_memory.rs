use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use super::shared::{
    ConfigValidationIssue, DEFAULT_SQLITE_FILE, default_loongclaw_home, expand_path,
    validate_numeric_range,
};

pub(crate) const MIN_MEMORY_SLIDING_WINDOW: usize = 1;
pub(crate) const MAX_MEMORY_SLIDING_WINDOW: usize = 128;
pub const DEFAULT_WEB_FETCH_MAX_BYTES: usize = 1024 * 1024;
pub const DEFAULT_WEB_FETCH_TIMEOUT_SECONDS: u64 = 15;
pub const DEFAULT_WEB_FETCH_MAX_REDIRECTS: usize = 3;
pub const DEFAULT_BROWSER_MAX_SESSIONS: usize = 8;
pub const DEFAULT_BROWSER_MAX_LINKS: usize = 40;
pub const DEFAULT_BROWSER_MAX_TEXT_CHARS: usize = 6000;
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
    pub approval: GovernedToolApprovalConfig,
    #[serde(default)]
    pub sessions: SessionToolConfig,
    #[serde(default)]
    pub messages: MessageToolConfig,
    #[serde(default)]
    pub delegate: DelegateToolConfig,
    #[serde(default)]
    pub browser: BrowserToolConfig,
    #[serde(default)]
    pub web: WebToolConfig,
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
    #[serde(default = "default_delegate_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_delegate_child_tool_allowlist")]
    pub child_tool_allowlist: Vec<String>,
    #[serde(default)]
    pub allow_shell_in_child: bool,
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

fn default_shell_default_mode() -> String {
    "deny".to_owned()
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: MemoryBackendKind,
    #[serde(default)]
    pub profile: MemoryProfile,
    #[serde(default)]
    pub system: MemorySystemKind,
    #[serde(default = "default_true")]
    pub fail_open: bool,
    #[serde(default)]
    pub ingest_mode: MemoryIngestMode,
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,
    #[serde(default = "default_sliding_window")]
    pub sliding_window: usize,
    #[serde(default = "default_summary_max_chars")]
    pub summary_max_chars: usize,
    #[serde(default)]
    pub profile_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBackendKind {
    #[default]
    Sqlite,
}

impl MemoryBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Some(Self::Sqlite),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProfile {
    #[default]
    WindowOnly,
    WindowPlusSummary,
    ProfilePlusWindow,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemorySystemKind {
    #[default]
    Builtin,
}

impl MemorySystemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "builtin" => Some(Self::Builtin),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for MemorySystemKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse_id(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unsupported memory.system `{}` (available: builtin)",
                raw.trim()
            ))
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryIngestMode {
    #[default]
    SyncMinimal,
    AsyncBackground,
}

impl MemoryIngestMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SyncMinimal => "sync_minimal",
            Self::AsyncBackground => "async_background",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sync_minimal" => Some(Self::SyncMinimal),
            "async_background" => Some(Self::AsyncBackground),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for MemoryIngestMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse_id(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unsupported memory.ingest_mode `{}` (available: sync_minimal, async_background)",
                raw.trim()
            ))
        })
    }
}

impl MemoryProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowOnly => "window_only",
            Self::WindowPlusSummary => "window_plus_summary",
            Self::ProfilePlusWindow => "profile_plus_window",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "window_only" => Some(Self::WindowOnly),
            "window_plus_summary" => Some(Self::WindowPlusSummary),
            "profile_plus_window" => Some(Self::ProfilePlusWindow),
            _ => None,
        }
    }

    pub const fn mode(self) -> MemoryMode {
        match self {
            Self::WindowOnly => MemoryMode::WindowOnly,
            Self::WindowPlusSummary => MemoryMode::WindowPlusSummary,
            Self::ProfilePlusWindow => MemoryMode::ProfilePlusWindow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemoryMode {
    #[default]
    WindowOnly,
    WindowPlusSummary,
    ProfilePlusWindow,
}

impl MemoryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowOnly => "window_only",
            Self::WindowPlusSummary => "window_plus_summary",
            Self::ProfilePlusWindow => "profile_plus_window",
        }
    }
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            shell_allow: default_shell_allow(),
            shell_deny: Vec::new(),
            shell_default_mode: default_shell_default_mode(),
            approval: GovernedToolApprovalConfig::default(),
            sessions: SessionToolConfig::default(),
            messages: MessageToolConfig::default(),
            delegate: DelegateToolConfig::default(),
            browser: BrowserToolConfig::default(),
            web: WebToolConfig::default(),
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
        }
    }
}

impl Default for DelegateToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_depth: default_delegate_max_depth(),
            timeout_seconds: default_delegate_timeout_seconds(),
            child_tool_allowlist: default_delegate_child_tool_allowlist(),
            allow_shell_in_child: false,
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

impl ToolConfig {
    pub fn resolved_file_root(&self) -> PathBuf {
        if let Some(path) = self.file_root.as_deref() {
            return expand_path(path);
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
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
        issues
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

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackendKind::default(),
            profile: MemoryProfile::default(),
            system: MemorySystemKind::default(),
            fail_open: default_true(),
            ingest_mode: MemoryIngestMode::default(),
            sqlite_path: default_sqlite_path(),
            sliding_window: default_sliding_window(),
            summary_max_chars: default_summary_max_chars(),
            profile_note: None,
        }
    }
}

impl MemoryConfig {
    pub fn resolved_sqlite_path(&self) -> PathBuf {
        expand_path(&self.sqlite_path)
    }

    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        if let Err(issue) = validate_numeric_range(
            "memory.sliding_window",
            self.sliding_window,
            MIN_MEMORY_SLIDING_WINDOW,
            MAX_MEMORY_SLIDING_WINDOW,
        ) {
            issues.push(*issue);
        }
        issues
    }

    pub const fn resolved_backend(&self) -> MemoryBackendKind {
        self.backend
    }

    pub const fn resolved_profile(&self) -> MemoryProfile {
        self.profile
    }

    pub const fn resolved_system(&self) -> MemorySystemKind {
        self.system
    }

    pub const fn resolved_mode(&self) -> MemoryMode {
        self.profile.mode()
    }

    pub const fn strict_mode_requested(&self) -> bool {
        !self.fail_open
    }

    pub const fn strict_mode_active(&self) -> bool {
        false
    }

    pub const fn effective_fail_open(&self) -> bool {
        !self.strict_mode_active()
    }

    pub fn summary_char_budget(&self) -> usize {
        self.summary_max_chars.max(256)
    }

    pub fn trimmed_profile_note(&self) -> Option<String> {
        self.profile_note
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }
}

fn default_sqlite_path() -> String {
    default_loongclaw_home()
        .join(DEFAULT_SQLITE_FILE)
        .display()
        .to_string()
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
    vec!["file.read".to_owned(), "file.write".to_owned()]
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

const fn default_require_download_approval() -> bool {
    true
}

const fn default_true() -> bool {
    true
}

const fn default_auto_expose_installed() -> bool {
    true
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
const fn default_sliding_window() -> usize {
    12
}

const fn default_summary_max_chars() -> usize {
    1200
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::DEFAULT_MEMORY_SYSTEM_ID;

    #[test]
    fn tool_config_defaults_expose_session_runtime_policy() {
        let config = ToolConfig::default();
        assert!(config.shell_allow.is_empty());
        assert!(config.shell_deny.is_empty());
        assert_eq!(config.shell_default_mode, "deny");
        assert_eq!(config.approval.mode, GovernedToolApprovalMode::Disabled);
        assert!(config.approval.approved_calls.is_empty());
        assert!(config.approval.denied_calls.is_empty());
        assert!(config.sessions.enabled);
        assert_eq!(config.sessions.visibility, SessionVisibility::Children);
        assert_eq!(config.sessions.list_limit, 100);
        assert_eq!(config.sessions.history_limit, 200);
        assert!(!config.messages.enabled);
        assert!(config.delegate.enabled);
        assert_eq!(config.delegate.max_depth, 1);
        assert_eq!(config.delegate.timeout_seconds, 60);
        assert_eq!(
            config.delegate.child_tool_allowlist,
            vec!["file.read".to_owned(), "file.write".to_owned()]
        );
        assert!(!config.delegate.allow_shell_in_child);
        assert!(config.browser.enabled);
        assert_eq!(config.browser.max_sessions, 8);
        assert_eq!(config.browser.max_links, 40);
        assert_eq!(config.browser.max_text_chars, 6000);
        assert!(config.web.enabled);
        assert!(!config.web.allow_private_hosts);
        assert!(config.web.allowed_domains.is_empty());
        assert!(config.web.blocked_domains.is_empty());
        assert_eq!(config.web.timeout_seconds, 15);
        assert_eq!(config.web.max_bytes, 1_048_576);
        assert_eq!(config.web.max_redirects, 3);
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

[tools.messages]
enabled = true

[tools.delegate]
enabled = false
max_depth = 2
timeout_seconds = 90
allow_shell_in_child = true
child_tool_allowlist = ["file.read", "shell.exec"]
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
        assert!(parsed.tools.messages.enabled);
        assert!(!parsed.tools.delegate.enabled);
        assert_eq!(parsed.tools.delegate.max_depth, 2);
        assert_eq!(parsed.tools.delegate.timeout_seconds, 90);
        assert!(parsed.tools.delegate.allow_shell_in_child);
        assert_eq!(
            parsed.tools.delegate.child_tool_allowlist,
            vec!["file.read".to_owned(), "shell.exec".to_owned()]
        );
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

    #[test]
    fn memory_profile_defaults_to_window_only() {
        let config = MemoryConfig::default();
        assert_eq!(config.backend, MemoryBackendKind::Sqlite);
        assert_eq!(config.profile, MemoryProfile::WindowOnly);
        assert_eq!(config.resolved_mode(), MemoryMode::WindowOnly);
    }

    #[test]
    fn memory_system_defaults_to_builtin() {
        let config = MemoryConfig::default();
        assert_eq!(config.system, MemorySystemKind::Builtin);
        assert_eq!(config.resolved_system(), MemorySystemKind::Builtin);
        assert_eq!(config.resolved_system().as_str(), DEFAULT_MEMORY_SYSTEM_ID);
    }

    #[test]
    fn memory_system_rejects_unimplemented_future_variant_ids() {
        assert_eq!(MemorySystemKind::parse_id("lucid"), None);
    }

    #[test]
    fn hydrated_memory_policy_defaults_are_fail_open_and_sync_minimal() {
        let config = MemoryConfig::default();
        assert!(config.fail_open);
        assert!(config.effective_fail_open());
        assert!(!config.strict_mode_requested());
        assert!(!config.strict_mode_active());
        assert_eq!(config.ingest_mode, MemoryIngestMode::SyncMinimal);
    }

    #[test]
    fn strict_mode_request_remains_reserved_and_disabled_by_default() {
        let config = MemoryConfig {
            fail_open: false,
            ..MemoryConfig::default()
        };

        assert!(config.strict_mode_requested());
        assert!(!config.strict_mode_active());
        assert!(config.effective_fail_open());
    }

    #[test]
    fn profile_plus_window_keeps_trimmed_profile_note() {
        let config = MemoryConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            profile_note: Some("  imported preferences  ".to_owned()),
            ..MemoryConfig::default()
        };

        assert_eq!(
            config.trimmed_profile_note().as_deref(),
            Some("imported preferences")
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
        assert!(config.auto_expose_installed);
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
