use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::shared::{
    ConfigValidationIssue, DEFAULT_SQLITE_FILE, default_loongclaw_home, expand_path,
    validate_numeric_range,
};

pub(crate) const MIN_MEMORY_SLIDING_WINDOW: usize = 1;
pub(crate) const MAX_MEMORY_SLIDING_WINDOW: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(default = "default_shell_allowlist")]
    pub shell_allowlist: Vec<String>,
    #[serde(default)]
    pub file_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: MemoryBackendKind,
    #[serde(default)]
    pub profile: MemoryProfile,
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
            shell_allowlist: default_shell_allowlist(),
            file_root: None,
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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackendKind::default(),
            profile: MemoryProfile::default(),
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

fn default_shell_allowlist() -> Vec<String> {
    vec![
        "echo".to_owned(),
        "cat".to_owned(),
        "ls".to_owned(),
        "pwd".to_owned(),
    ]
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

    #[test]
    fn memory_profile_defaults_to_window_only() {
        let config = MemoryConfig::default();
        assert_eq!(config.backend, MemoryBackendKind::Sqlite);
        assert_eq!(config.profile, MemoryProfile::WindowOnly);
        assert_eq!(config.resolved_mode(), MemoryMode::WindowOnly);
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
}
