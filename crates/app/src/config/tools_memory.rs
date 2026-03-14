use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use super::shared::{
    ConfigValidationIssue, DEFAULT_SQLITE_FILE, default_loongclaw_home, expand_path,
    validate_numeric_range,
};

pub(crate) const MIN_MEMORY_SLIDING_WINDOW: usize = 1;
pub(crate) const MAX_MEMORY_SLIDING_WINDOW: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: MemoryBackendKind,
    #[serde(default)]
    pub profile: MemoryProfile,
    #[serde(default)]
    pub system: MemorySystemKind,
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

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            file_root: None,
            shell_allow: default_shell_allow(),
            shell_deny: Vec::new(),
            shell_default_mode: default_shell_default_mode(),
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
        Vec::new()
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

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackendKind::default(),
            profile: MemoryProfile::default(),
            system: MemorySystemKind::default(),
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

const fn default_require_download_approval() -> bool {
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
}
