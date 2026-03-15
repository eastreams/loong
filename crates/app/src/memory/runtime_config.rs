use std::path::PathBuf;
use std::sync::OnceLock;

use crate::config::{
    MemoryBackendKind, MemoryConfig, MemoryIngestMode, MemoryMode, MemoryProfile, MemorySystemKind,
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
    pub mode: MemoryMode,
    pub fail_open: bool,
    pub ingest_mode: MemoryIngestMode,
    pub sqlite_path: Option<PathBuf>,
    pub sliding_window: usize,
    pub summary_max_chars: usize,
    pub profile_note: Option<String>,
}

impl Default for MemoryRuntimeConfig {
    fn default() -> Self {
        let defaults = MemoryConfig::default();
        Self {
            backend: defaults.backend,
            profile: defaults.profile,
            system: defaults.system,
            mode: defaults.resolved_mode(),
            fail_open: defaults.fail_open,
            ingest_mode: defaults.ingest_mode,
            sqlite_path: None,
            sliding_window: defaults.sliding_window,
            summary_max_chars: defaults.summary_char_budget(),
            profile_note: defaults.trimmed_profile_note(),
        }
    }
}

impl MemoryRuntimeConfig {
    fn from_memory_config_base(config: &MemoryConfig) -> Self {
        Self {
            backend: config.resolved_backend(),
            profile: config.resolved_profile(),
            system: config.resolved_system(),
            mode: config.resolved_mode(),
            fail_open: config.fail_open,
            ingest_mode: config.ingest_mode,
            sqlite_path: Some(config.resolved_sqlite_path()),
            sliding_window: config.sliding_window,
            summary_max_chars: config.summary_char_budget(),
            profile_note: config.trimmed_profile_note(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Some(backend) = std::env::var("LOONGCLAW_MEMORY_BACKEND")
            .ok()
            .as_deref()
            .and_then(MemoryBackendKind::parse_id)
        {
            self.backend = backend;
        }

        if let Some(profile) = std::env::var("LOONGCLAW_MEMORY_PROFILE")
            .ok()
            .as_deref()
            .and_then(MemoryProfile::parse_id)
        {
            self.profile = profile;
            self.mode = profile.mode();
        }

        if let Some(system) = crate::memory::supported_memory_system_kind_from_env() {
            self.system = system;
        }

        if let Some(fail_open) = parse_bool(std::env::var("LOONGCLAW_MEMORY_FAIL_OPEN").ok()) {
            self.fail_open = fail_open;
        }

        if let Some(ingest_mode) = std::env::var("LOONGCLAW_MEMORY_INGEST_MODE")
            .ok()
            .as_deref()
            .and_then(MemoryIngestMode::parse_id)
        {
            self.ingest_mode = ingest_mode;
        }

        if let Some(sqlite_path) = std::env::var("LOONGCLAW_SQLITE_PATH")
            .ok()
            .map(PathBuf::from)
        {
            self.sqlite_path = Some(sqlite_path);
        }

        if let Some(sliding_window) =
            parse_positive_usize(std::env::var("LOONGCLAW_SLIDING_WINDOW").ok())
        {
            self.sliding_window = sliding_window;
        }

        if let Some(summary_max_chars) =
            parse_positive_usize(std::env::var("LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS").ok())
        {
            self.summary_max_chars = summary_max_chars;
        }

        if let Ok(profile_note) = std::env::var("LOONGCLAW_MEMORY_PROFILE_NOTE") {
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
    /// `LOONGCLAW_SQLITE_PATH`.
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

    pub const fn strict_mode_requested(&self) -> bool {
        !self.fail_open
    }

    pub const fn strict_mode_active(&self) -> bool {
        false
    }

    pub const fn effective_fail_open(&self) -> bool {
        !self.strict_mode_active()
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
    use crate::test_support::ScopedEnv;

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
            mode: MemoryMode::WindowOnly,
            fail_open: true,
            ingest_mode: MemoryIngestMode::SyncMinimal,
            sqlite_path: Some(PathBuf::from("/tmp/test-memory.sqlite3")),
            sliding_window: 12,
            summary_max_chars: 1200,
            profile_note: None,
        };
        assert_eq!(
            config.sqlite_path,
            Some(PathBuf::from("/tmp/test-memory.sqlite3"))
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
        env.set("LOONGCLAW_MEMORY_PROFILE", "profile_plus_window");
        env.set("LOONGCLAW_MEMORY_FAIL_OPEN", "true");
        env.set("LOONGCLAW_MEMORY_INGEST_MODE", "async_background");
        env.set("LOONGCLAW_SLIDING_WINDOW", "24");
        env.set("LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS", "2048");
        env.set("LOONGCLAW_MEMORY_PROFILE_NOTE", "  env profile note  ");
        env.set("LOONGCLAW_SQLITE_PATH", "/tmp/env-memory.sqlite3");

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
        assert_eq!(runtime.profile_note.as_deref(), Some("env profile note"));
    }
}
