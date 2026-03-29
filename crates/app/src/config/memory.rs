use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use super::shared::{
    ConfigValidationIssue, DEFAULT_SQLITE_FILE, default_loongclaw_home, expand_path,
    validate_numeric_range,
};

pub(crate) const MIN_MEMORY_SLIDING_WINDOW: usize = 1;
pub(crate) const MAX_MEMORY_SLIDING_WINDOW: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: MemoryBackendKind,
    #[serde(default)]
    pub profile: MemoryProfile,
    #[serde(default)]
    pub system: MemorySystemKind,
    #[serde(default, deserialize_with = "deserialize_memory_system_id")]
    pub system_id: Option<String>,
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

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackendKind::default(),
            profile: MemoryProfile::default(),
            system: MemorySystemKind::default(),
            system_id: None,
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

    pub fn resolved_system_id(&self) -> String {
        self.system_id
            .clone()
            .unwrap_or_else(|| self.system.as_str().to_owned())
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

fn deserialize_memory_system_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.and_then(|value| crate::memory::normalize_system_id(value.as_str())))
}

fn default_sqlite_path() -> String {
    default_loongclaw_home()
        .join(DEFAULT_SQLITE_FILE)
        .display()
        .to_string()
}

const fn default_true() -> bool {
    true
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
    use serde_json::json;

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
    fn memory_system_field_accepts_registry_backed_string_ids() {
        let raw = json!({
            "system_id": "Lucid"
        });

        let config: MemoryConfig =
            serde_json::from_value(raw).expect("registry-backed memory.system should deserialize");

        assert_eq!(config.system_id.as_deref(), Some("lucid"));
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
}
