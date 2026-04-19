use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use super::shared::{
    ConfigValidationIssue, DEFAULT_SQLITE_FILE, default_loong_home, expand_path,
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
    #[serde(default, deserialize_with = "deserialize_memory_agent_id")]
    pub agent_id: Option<String>,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_personalization_config"
    )]
    pub personalization: Option<PersonalizationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonalizationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_density: Option<ResponseDensity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative_level: Option<InitiativeLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing_boundaries: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "personalization_prompt_state_is_pending"
    )]
    pub prompt_state: PersonalizationPromptState,
    #[serde(
        default = "default_personalization_schema_version",
        skip_serializing_if = "is_default_personalization_schema_version"
    )]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_epoch_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseDensity {
    Concise,
    Balanced,
    Thorough,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InitiativeLevel {
    AskBeforeActing,
    Balanced,
    HighInitiative,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PersonalizationPromptState {
    #[default]
    Pending,
    Deferred,
    Suppressed,
    Configured,
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

impl PersonalizationConfig {
    pub fn normalized(&self) -> Option<Self> {
        let preferred_name = trim_optional_text(self.preferred_name.as_deref());
        let response_density = self.response_density;
        let initiative_level = self.initiative_level;
        let standing_boundaries = trim_optional_text(self.standing_boundaries.as_deref());
        let timezone = trim_optional_text(self.timezone.as_deref());
        let locale = trim_optional_text(self.locale.as_deref());
        let prompt_state = self.prompt_state;
        let schema_version = normalize_personalization_schema_version(self.schema_version);
        let updated_at_epoch_seconds = self.updated_at_epoch_seconds;

        let is_meaningful = preferred_name.is_some()
            || response_density.is_some()
            || initiative_level.is_some()
            || standing_boundaries.is_some()
            || timezone.is_some()
            || locale.is_some()
            || !prompt_state.is_pending();
        if !is_meaningful {
            return None;
        }

        Some(Self {
            preferred_name,
            response_density,
            initiative_level,
            standing_boundaries,
            timezone,
            locale,
            prompt_state,
            schema_version,
            updated_at_epoch_seconds,
        })
    }

    pub fn has_operator_preferences(&self) -> bool {
        self.normalized().is_some_and(|personalization| {
            personalization.preferred_name.is_some()
                || personalization.response_density.is_some()
                || personalization.initiative_level.is_some()
                || personalization.standing_boundaries.is_some()
                || personalization.timezone.is_some()
                || personalization.locale.is_some()
        })
    }

    pub fn suppresses_suggestions(&self) -> bool {
        self.normalized()
            .is_some_and(|personalization| personalization.prompt_state.is_suppressed())
    }
}

impl Default for PersonalizationConfig {
    fn default() -> Self {
        Self {
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: PersonalizationPromptState::default(),
            schema_version: default_personalization_schema_version(),
            updated_at_epoch_seconds: None,
        }
    }
}

impl ResponseDensity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Balanced => "balanced",
            Self::Thorough => "thorough",
        }
    }
}

impl InitiativeLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AskBeforeActing => "ask_before_acting",
            Self::Balanced => "balanced",
            Self::HighInitiative => "high_initiative",
        }
    }
}

impl PersonalizationPromptState {
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }

    pub const fn is_suppressed(self) -> bool {
        matches!(self, Self::Suppressed)
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
    WorkspaceRecall,
}

impl MemorySystemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::WorkspaceRecall => "workspace_recall",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "builtin" => Some(Self::Builtin),
            "workspace_recall" => Some(Self::WorkspaceRecall),
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
                "unsupported memory.system `{}` (available: builtin, workspace_recall)",
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
            agent_id: None,
            fail_open: default_true(),
            ingest_mode: MemoryIngestMode::default(),
            sqlite_path: default_sqlite_path(),
            sliding_window: default_sliding_window(),
            summary_max_chars: default_summary_max_chars(),
            profile_note: None,
            personalization: None,
        }
    }
}

impl MemoryConfig {
    pub fn resolved_sqlite_path(&self) -> PathBuf {
        let base_path = expand_path(&self.sqlite_path);
        scoped_sqlite_path_for_agent(base_path.as_path(), self.normalized_agent_id().as_deref())
    }

    pub fn normalized_agent_id(&self) -> Option<String> {
        normalize_memory_agent_id(self.agent_id.clone())
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

    pub fn trimmed_personalization(&self) -> Option<PersonalizationConfig> {
        let personalization = self.personalization.as_ref()?;
        personalization.normalized()
    }
}

fn deserialize_memory_system_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.and_then(|value| crate::memory::normalize_system_id(value.as_str())))
}

fn deserialize_memory_agent_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(normalize_memory_agent_id(raw))
}

fn deserialize_personalization_config<'de, D>(
    deserializer: D,
) -> Result<Option<PersonalizationConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<PersonalizationConfig>::deserialize(deserializer)?;
    Ok(raw.and_then(|personalization| personalization.normalized()))
}

fn trim_optional_text(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    Some(trimmed_value.to_owned())
}

pub(crate) fn normalize_memory_agent_id(raw: Option<String>) -> Option<String> {
    raw.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn scoped_sqlite_path_for_agent(base_path: &Path, agent_id: Option<&str>) -> PathBuf {
    let Some(agent_id) = agent_id else {
        return base_path.to_path_buf();
    };

    let sanitized_agent_id = sanitize_memory_agent_id_for_path(agent_id);
    if sanitized_agent_id.is_empty() {
        return base_path.to_path_buf();
    }

    let base_parent = base_path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = base_path
        .file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| std::ffi::OsString::from(DEFAULT_SQLITE_FILE));

    base_parent
        .join("agents")
        .join(sanitized_agent_id)
        .join(file_name)
}

pub(crate) fn scoped_memory_workspace_root_for_agent(
    base_workspace_root: &Path,
    agent_id: Option<&str>,
) -> PathBuf {
    let Some(agent_id) = agent_id else {
        return base_workspace_root.to_path_buf();
    };

    let sanitized_agent_id = sanitize_memory_agent_id_for_path(agent_id);
    if sanitized_agent_id.is_empty() {
        return base_workspace_root.to_path_buf();
    }

    base_workspace_root
        .join(".loong")
        .join("agents")
        .join(sanitized_agent_id)
}

fn sanitize_memory_agent_id_for_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let sanitized = trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();

    let already_path_safe = !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    if already_path_safe {
        return trimmed.to_owned();
    }

    let digest = Sha256::digest(trimmed.as_bytes());
    let hash = hex::encode(digest.iter().take(6).copied().collect::<Vec<_>>());
    if sanitized.is_empty() {
        return format!("agent_{hash}");
    }

    format!("{sanitized}_{hash}")
}

const fn default_personalization_schema_version() -> u32 {
    1
}

const fn normalize_personalization_schema_version(raw: u32) -> u32 {
    if raw == 0 {
        return default_personalization_schema_version();
    }
    raw
}

fn is_default_personalization_schema_version(raw: &u32) -> bool {
    *raw == default_personalization_schema_version()
}

fn personalization_prompt_state_is_pending(state: &PersonalizationPromptState) -> bool {
    state.is_pending()
}

fn default_sqlite_path() -> String {
    default_loong_home()
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
    use crate::test_support::ScopedEnv;
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
    fn memory_system_accepts_workspace_recall_variant() {
        assert_eq!(
            MemorySystemKind::parse_id("workspace_recall"),
            Some(MemorySystemKind::WorkspaceRecall)
        );
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

    #[test]
    fn memory_agent_id_is_normalized_when_deserialized() {
        let raw = json!({
            "agent_id": "  health  "
        });

        let config: MemoryConfig =
            serde_json::from_value(raw).expect("memory.agent_id should deserialize");

        assert_eq!(config.agent_id.as_deref(), Some("health"));
        assert_eq!(config.normalized_agent_id().as_deref(), Some("health"));
    }

    #[test]
    fn resolved_sqlite_path_scopes_by_agent_id() {
        let mut env = ScopedEnv::new();
        env.remove("LOONG_SQLITE_PATH");
        env.remove("LOONG_MEMORY_AGENT_ID");

        let config = MemoryConfig {
            sqlite_path: "/tmp/loong/memory.sqlite3".to_owned(),
            agent_id: Some("investment".to_owned()),
            ..MemoryConfig::default()
        };

        assert_eq!(
            config.resolved_sqlite_path(),
            PathBuf::from("/tmp/loong/agents/investment/memory.sqlite3")
        );
    }

    #[test]
    fn resolved_sqlite_path_ignores_env_overrides() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONG_SQLITE_PATH", "/tmp/env-memory.sqlite3");
        env.set("LOONG_MEMORY_AGENT_ID", "health");

        let config = MemoryConfig {
            sqlite_path: "/tmp/config-memory.sqlite3".to_owned(),
            ..MemoryConfig::default()
        };

        assert_eq!(
            config.resolved_sqlite_path(),
            PathBuf::from("/tmp/config-memory.sqlite3")
        );
    }

    #[test]
    fn scoped_sqlite_paths_avoid_collisions_for_different_unsafe_agent_ids() {
        let path_a =
            scoped_sqlite_path_for_agent(Path::new("/tmp/loong/memory.sqlite3"), Some("topic/a"));
        let path_b =
            scoped_sqlite_path_for_agent(Path::new("/tmp/loong/memory.sqlite3"), Some("topic?a"));

        assert_ne!(path_a, path_b);
    }

    #[test]
    fn personalization_trims_string_fields_and_preserves_non_default_state() {
        let config = MemoryConfig {
            personalization: Some(PersonalizationConfig {
                preferred_name: Some("  Chum  ".to_owned()),
                response_density: Some(ResponseDensity::Balanced),
                initiative_level: None,
                standing_boundaries: Some("  Ask before destructive actions.  ".to_owned()),
                timezone: Some("  Asia/Shanghai  ".to_owned()),
                locale: Some("  zh-CN  ".to_owned()),
                prompt_state: PersonalizationPromptState::Deferred,
                schema_version: 0,
                updated_at_epoch_seconds: Some(7),
            }),
            ..MemoryConfig::default()
        };

        let personalization = config
            .trimmed_personalization()
            .expect("personalization should stay present");

        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(
            personalization.standing_boundaries.as_deref(),
            Some("Ask before destructive actions.")
        );
        assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));
        assert_eq!(
            personalization.response_density,
            Some(ResponseDensity::Balanced)
        );
        assert_eq!(
            personalization.prompt_state,
            PersonalizationPromptState::Deferred
        );
        assert_eq!(personalization.schema_version, 1);
        assert_eq!(personalization.updated_at_epoch_seconds, Some(7));
    }

    #[test]
    fn personalization_drops_empty_payload_with_only_default_metadata() {
        let config = MemoryConfig {
            personalization: Some(PersonalizationConfig {
                preferred_name: Some("   ".to_owned()),
                response_density: None,
                initiative_level: None,
                standing_boundaries: Some("\n".to_owned()),
                timezone: None,
                locale: None,
                prompt_state: PersonalizationPromptState::Pending,
                schema_version: 1,
                updated_at_epoch_seconds: Some(7),
            }),
            ..MemoryConfig::default()
        };

        assert_eq!(config.trimmed_personalization(), None);
    }
}
