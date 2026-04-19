use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::{
    MemoryBackendKind, MemoryConfig, MemoryIngestMode, MemoryMode, MemoryProfile, MemorySystemKind,
    PersonalizationConfig,
};

/// Typed runtime configuration for the memory (SQLite) subsystem.
///
/// Mirrors [`crate::tools::runtime_config::ToolRuntimeConfig`] — a
/// process-wide singleton populated once at startup so that per-call
/// `std::env::var` lookups are avoided on the hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRuntimeConfig {
    pub backend: MemoryBackendKind,
    pub profile: MemoryProfile,
    pub system: MemorySystemKind,
    pub resolved_system_id: Option<String>,
    pub agent_id: Option<String>,
    pub mode: MemoryMode,
    pub fail_open: bool,
    pub ingest_mode: MemoryIngestMode,
    pub sqlite_path: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub sliding_window: usize,
    pub summary_max_chars: usize,
    pub profile_note: Option<String>,
    pub personalization: Option<PersonalizationConfig>,
}

impl Default for MemoryRuntimeConfig {
    fn default() -> Self {
        let defaults = MemoryConfig::default();
        Self {
            backend: defaults.backend,
            profile: defaults.profile,
            system: defaults.system,
            resolved_system_id: Some(defaults.resolved_system_id()),
            agent_id: defaults.normalized_agent_id(),
            mode: defaults.resolved_mode(),
            fail_open: defaults.fail_open,
            ingest_mode: defaults.ingest_mode,
            sqlite_path: None,
            workspace_root: None,
            sliding_window: defaults.sliding_window,
            summary_max_chars: defaults.summary_char_budget(),
            profile_note: defaults.trimmed_profile_note(),
            personalization: defaults.trimmed_personalization(),
        }
    }
}

impl MemoryRuntimeConfig {
    fn from_memory_config_base(config: &MemoryConfig) -> Self {
        Self {
            backend: config.resolved_backend(),
            profile: config.resolved_profile(),
            system: config.resolved_system(),
            resolved_system_id: Some(config.resolved_system_id()),
            agent_id: config.normalized_agent_id(),
            mode: config.resolved_mode(),
            fail_open: config.fail_open,
            ingest_mode: config.ingest_mode,
            sqlite_path: Some(crate::config::expand_path(&config.sqlite_path)),
            workspace_root: None,
            sliding_window: config.sliding_window,
            summary_max_chars: config.summary_char_budget(),
            profile_note: config.trimmed_profile_note(),
            personalization: config.trimmed_personalization(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Some(backend) = std::env::var("LOONG_MEMORY_BACKEND")
            .ok()
            .as_deref()
            .and_then(MemoryBackendKind::parse_id)
        {
            self.backend = backend;
        }

        if let Some(profile) = std::env::var("LOONG_MEMORY_PROFILE")
            .ok()
            .as_deref()
            .and_then(MemoryProfile::parse_id)
        {
            self.profile = profile;
            self.mode = profile.mode();
        }

        if let Some(system_id) = crate::memory::registered_memory_system_id_from_env() {
            self.system = MemorySystemKind::parse_id(system_id.as_str()).unwrap_or_default();
            self.resolved_system_id = Some(system_id);
        }

        if let Ok(agent_id) = std::env::var("LOONG_MEMORY_AGENT_ID") {
            self.agent_id = crate::config::normalize_memory_agent_id(Some(agent_id));
        }

        if let Some(fail_open) = parse_bool(std::env::var("LOONG_MEMORY_FAIL_OPEN").ok()) {
            self.fail_open = fail_open;
        }

        if let Some(ingest_mode) = std::env::var("LOONG_MEMORY_INGEST_MODE")
            .ok()
            .as_deref()
            .and_then(MemoryIngestMode::parse_id)
        {
            self.ingest_mode = ingest_mode;
        }

        if let Some(sqlite_path) = std::env::var("LOONG_SQLITE_PATH").ok().map(PathBuf::from) {
            self.sqlite_path = Some(sqlite_path);
        }

        if let Some(sliding_window) =
            parse_positive_usize(std::env::var("LOONG_SLIDING_WINDOW").ok())
        {
            self.sliding_window = sliding_window;
        }

        if let Some(summary_max_chars) =
            parse_positive_usize(std::env::var("LOONG_MEMORY_SUMMARY_MAX_CHARS").ok())
        {
            self.summary_max_chars = summary_max_chars;
        }

        if let Ok(profile_note) = std::env::var("LOONG_MEMORY_PROFILE_NOTE") {
            let trimmed = profile_note.trim();
            self.profile_note = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
    }

    /// Build a config by reading the legacy environment variable.
    ///
    /// Keeps full backward compatibility for callers that still rely on
    /// `LOONG_SQLITE_PATH`.
    pub fn from_env() -> Self {
        let defaults = MemoryConfig::default();
        let mut runtime = Self::from_memory_config_base(&defaults);
        runtime.apply_env_overrides();
        runtime
    }

    pub fn from_memory_config(config: &MemoryConfig) -> Self {
        let mut runtime = Self::from_memory_config_base(config);
        runtime.apply_env_overrides();
        runtime
    }

    pub fn from_memory_config_without_env_overrides(config: &MemoryConfig) -> Self {
        Self::from_memory_config_base(config)
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

    pub fn with_agent_id(mut self, agent_id: Option<&str>) -> Self {
        self.agent_id = crate::config::normalize_memory_agent_id(agent_id.map(str::to_owned));
        self
    }

    pub fn with_workspace_root(mut self, workspace_root: Option<PathBuf>) -> Self {
        self.workspace_root = workspace_root;
        self
    }

    pub fn effective_workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    pub fn resolved_workspace_root_from(&self, base_workspace_root: &Path) -> PathBuf {
        crate::config::scoped_memory_workspace_root_for_agent(
            base_workspace_root,
            self.agent_id.as_deref(),
        )
    }

    pub fn resolved_sqlite_path(&self) -> PathBuf {
        let base_path = self
            .sqlite_path
            .clone()
            .unwrap_or_else(|| crate::config::default_loong_home().join("memory.sqlite3"));
        crate::config::scoped_sqlite_path_for_agent(base_path.as_path(), self.agent_id.as_deref())
    }

    pub fn selected_system_id(&self) -> &str {
        self.resolved_system_id
            .as_deref()
            .unwrap_or(crate::memory::DEFAULT_MEMORY_SYSTEM_ID)
    }
}

fn parse_positive_usize(raw: Option<String>) -> Option<usize> {
    raw.and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn parse_bool(raw: Option<String>) -> Option<bool> {
    raw.and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

static MEMORY_RUNTIME_CONFIG: OnceLock<MemoryRuntimeConfig> = OnceLock::new();

/// Initialise the process-wide memory runtime config.
///
/// Returns `Ok(())` on the first call.  Subsequent calls return
/// `Err` because the `OnceLock` rejects duplicate initialisation.
pub fn init_memory_runtime_config(config: MemoryRuntimeConfig) -> Result<(), String> {
    MEMORY_RUNTIME_CONFIG.set(config).map_err(|_err| {
        "memory runtime config already initialised (duplicate init_memory_runtime_config call)"
            .to_owned()
    })
}

/// Return the process-wide memory runtime config.
///
/// If `init_memory_runtime_config` was never called the config is lazily
/// populated from environment variables (backward-compat path).
pub fn get_memory_runtime_config() -> &'static MemoryRuntimeConfig {
    MEMORY_RUNTIME_CONFIG.get_or_init(MemoryRuntimeConfig::from_env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{
        MEMORY_SYSTEM_ENV, MemorySystem, MemorySystemCapability, MemorySystemMetadata,
        register_memory_system,
    };
    use crate::test_support::ScopedEnv;

    struct RuntimeConfigRegistryMemorySystem;

    impl MemorySystem for RuntimeConfigRegistryMemorySystem {
        fn id(&self) -> &'static str {
            "registry-runtime-config"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-runtime-config",
                [MemorySystemCapability::PromptHydration],
                "Runtime config registry test system",
            )
        }
    }

    #[test]
    fn parse_sliding_window_accepts_positive_integer() {
        assert_eq!(parse_positive_usize(Some("24".to_owned())), Some(24));
    }

    #[test]
    fn parse_sliding_window_rejects_zero_negative_and_invalid_values() {
        assert_eq!(parse_positive_usize(Some("0".to_owned())), None);
        assert_eq!(parse_positive_usize(Some("-1".to_owned())), None);
        assert_eq!(parse_positive_usize(Some("invalid".to_owned())), None);
    }

    #[test]
    fn parse_sliding_window_returns_none_when_absent() {
        assert_eq!(parse_positive_usize(None), None);
    }

    #[test]
    fn parse_bool_accepts_common_true_false_forms() {
        assert_eq!(parse_bool(Some("true".to_owned())), Some(true));
        assert_eq!(parse_bool(Some("1".to_owned())), Some(true));
        assert_eq!(parse_bool(Some("off".to_owned())), Some(false));
        assert_eq!(parse_bool(Some("0".to_owned())), Some(false));
    }

    #[test]
    fn parse_bool_rejects_unknown_values() {
        assert_eq!(parse_bool(Some("maybe".to_owned())), None);
        assert_eq!(parse_bool(None), None);
    }

    #[test]
    fn memory_runtime_config_default_has_no_path() {
        let config = MemoryRuntimeConfig::default();
        assert!(config.sqlite_path.is_none());
    }

    #[test]
    fn explicit_path_overrides_default() {
        let config = MemoryRuntimeConfig {
            backend: MemoryBackendKind::Sqlite,
            profile: MemoryProfile::WindowOnly,
            system: MemorySystemKind::Builtin,
            resolved_system_id: Some(crate::memory::DEFAULT_MEMORY_SYSTEM_ID.to_owned()),
            agent_id: None,
            mode: MemoryMode::WindowOnly,
            fail_open: true,
            ingest_mode: MemoryIngestMode::SyncMinimal,
            sqlite_path: Some(PathBuf::from("/tmp/test-memory.sqlite3")),
            workspace_root: None,
            sliding_window: 12,
            summary_max_chars: 1200,
            profile_note: None,
            personalization: None,
        };
        assert_eq!(
            config.sqlite_path,
            Some(PathBuf::from("/tmp/test-memory.sqlite3"))
        );
        assert_eq!(
            config.resolved_sqlite_path(),
            PathBuf::from("/tmp/test-memory.sqlite3")
        );
    }

    #[test]
    fn runtime_config_from_memory_config_carries_profile_and_limits() {
        let _env = ScopedEnv::new();
        let config = MemoryConfig {
            profile: MemoryProfile::WindowPlusSummary,
            summary_max_chars: 900,
            ..MemoryConfig::default()
        };

        let runtime = MemoryRuntimeConfig::from_memory_config(&config);

        assert_eq!(runtime.backend, MemoryBackendKind::Sqlite);
        assert_eq!(runtime.profile, MemoryProfile::WindowPlusSummary);
        assert_eq!(runtime.mode, MemoryMode::WindowPlusSummary);
        assert_eq!(runtime.summary_max_chars, 900);
    }

    #[test]
    fn runtime_config_from_memory_config_carries_personalization() {
        let _env = ScopedEnv::new();
        let default_personalization = PersonalizationConfig::default();
        let schema_version = default_personalization.schema_version;
        let personalization = PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(crate::config::ResponseDensity::Balanced),
            initiative_level: Some(crate::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: Some("zh-CN".to_owned()),
            prompt_state: crate::config::PersonalizationPromptState::Suppressed,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        };
        let config = MemoryConfig {
            personalization: Some(personalization),
            ..MemoryConfig::default()
        };

        let runtime = MemoryRuntimeConfig::from_memory_config(&config);
        let expected_personalization = config.trimmed_personalization();

        assert_eq!(runtime.personalization, expected_personalization);
    }

    #[test]
    fn hydrated_memory_runtime_config_carries_system_policy() {
        let _env = ScopedEnv::new();
        let config = MemoryConfig {
            system: crate::config::MemorySystemKind::Builtin,
            fail_open: false,
            ingest_mode: crate::config::MemoryIngestMode::AsyncBackground,
            ..MemoryConfig::default()
        };

        let runtime = MemoryRuntimeConfig::from_memory_config(&config);

        assert_eq!(runtime.system, crate::config::MemorySystemKind::Builtin);
        assert_eq!(
            runtime.resolved_system_id.as_deref(),
            Some(crate::memory::DEFAULT_MEMORY_SYSTEM_ID)
        );
        assert!(!runtime.fail_open);
        assert!(runtime.strict_mode_requested());
        assert!(!runtime.strict_mode_active());
        assert!(runtime.effective_fail_open());
        assert_eq!(
            runtime.ingest_mode,
            crate::config::MemoryIngestMode::AsyncBackground
        );
    }

    #[test]
    fn runtime_config_from_memory_config_applies_memory_env_overrides() {
        let mut env = ScopedEnv::new();
        env.set("LOONG_MEMORY_PROFILE", "profile_plus_window");
        env.set("LOONG_MEMORY_FAIL_OPEN", "true");
        env.set("LOONG_MEMORY_INGEST_MODE", "async_background");
        env.set("LOONG_MEMORY_AGENT_ID", "health");
        env.set("LOONG_SLIDING_WINDOW", "24");
        env.set("LOONG_MEMORY_SUMMARY_MAX_CHARS", "2048");
        env.set("LOONG_MEMORY_PROFILE_NOTE", "  env profile note  ");
        env.set("LOONG_SQLITE_PATH", "/tmp/env-memory.sqlite3");

        let config = MemoryConfig {
            profile: MemoryProfile::WindowOnly,
            fail_open: false,
            ingest_mode: MemoryIngestMode::SyncMinimal,
            sliding_window: 12,
            summary_max_chars: 900,
            profile_note: Some("config note".to_owned()),
            ..MemoryConfig::default()
        };

        let runtime = MemoryRuntimeConfig::from_memory_config(&config);

        assert_eq!(runtime.profile, MemoryProfile::ProfilePlusWindow);
        assert_eq!(runtime.mode, MemoryMode::ProfilePlusWindow);
        assert!(runtime.fail_open);
        assert_eq!(runtime.ingest_mode, MemoryIngestMode::AsyncBackground);
        assert_eq!(runtime.sliding_window, 24);
        assert_eq!(runtime.summary_max_chars, 2048);
        assert_eq!(
            runtime.sqlite_path,
            Some(PathBuf::from("/tmp/env-memory.sqlite3"))
        );
        assert_eq!(runtime.agent_id.as_deref(), Some("health"));
        assert_eq!(
            runtime.resolved_sqlite_path(),
            PathBuf::from("/tmp/agents/health/env-memory.sqlite3")
        );
        assert_eq!(runtime.profile_note.as_deref(), Some("env profile note"));
    }

    #[test]
    fn runtime_config_resolves_agent_scoped_sqlite_path() {
        let config = MemoryRuntimeConfig {
            sqlite_path: Some(PathBuf::from("/tmp/loong/memory.sqlite3")),
            agent_id: Some("investment".to_owned()),
            ..MemoryRuntimeConfig::default()
        };

        assert_eq!(
            config.resolved_sqlite_path(),
            PathBuf::from("/tmp/loong/agents/investment/memory.sqlite3")
        );
    }

    #[test]
    fn runtime_config_workspace_root_override_is_reported() {
        let workspace_root = PathBuf::from("/tmp/topic-health");
        let config =
            MemoryRuntimeConfig::default().with_workspace_root(Some(workspace_root.clone()));

        assert_eq!(
            config.effective_workspace_root(),
            Some(workspace_root.as_path())
        );
    }

    #[test]
    fn runtime_config_resolves_agent_scoped_workspace_root() {
        let config = MemoryRuntimeConfig {
            agent_id: Some("health".to_owned()),
            ..MemoryRuntimeConfig::default()
        };

        assert_eq!(
            config.resolved_workspace_root_from(Path::new("/tmp/workspace")),
            PathBuf::from("/tmp/workspace/.loong/agents/health")
        );
    }

    #[test]
    fn memory_system_field_preserves_registry_backed_env_selection() {
        register_memory_system("registry-runtime-config", || {
            Box::new(RuntimeConfigRegistryMemorySystem)
        })
        .expect("register runtime-config registry system");
        let mut env = ScopedEnv::new();
        env.set(MEMORY_SYSTEM_ENV, "registry-runtime-config");

        let config = MemoryConfig::default();
        let runtime = MemoryRuntimeConfig::from_memory_config(&config);

        assert_eq!(
            runtime.resolved_system_id.as_deref(),
            Some("registry-runtime-config")
        );
    }
}
