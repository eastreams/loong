use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;
use crate::config::{
    LoongClawConfig, MemoryBackendKind, MemoryIngestMode, MemoryMode, MemoryProfile,
    MemorySystemKind,
};

use super::system::{
    BuiltinMemorySystem, DEFAULT_MEMORY_SYSTEM_ID, MemorySystem, MemorySystemMetadata,
};
use super::runtime_config::MemoryRuntimeConfig;

pub const MEMORY_SYSTEM_ENV: &str = "LOONGCLAW_MEMORY_SYSTEM";

type MemorySystemFactory = Arc<dyn Fn() -> Box<dyn MemorySystem> + Send + Sync>;

static MEMORY_SYSTEM_REGISTRY: OnceLock<RwLock<BTreeMap<String, MemorySystemFactory>>> =
    OnceLock::new();
#[cfg(test)]
static MEMORY_SYSTEM_ENV_OVERRIDE: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySystemSelectionSource {
    Env,
    Config,
    Default,
}

impl MemorySystemSelectionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySystemSelection {
    pub id: String,
    pub source: MemorySystemSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySystemRuntimeSnapshot {
    pub selected: MemorySystemSelection,
    pub selected_metadata: MemorySystemMetadata,
    pub available: Vec<MemorySystemMetadata>,
    pub policy: MemorySystemPolicySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySystemPolicySnapshot {
    pub backend: MemoryBackendKind,
    pub profile: MemoryProfile,
    pub mode: MemoryMode,
    pub ingest_mode: MemoryIngestMode,
    pub fail_open: bool,
    pub strict_mode_requested: bool,
    pub strict_mode_active: bool,
    pub effective_fail_open: bool,
}

impl MemorySystemPolicySnapshot {
    fn from_runtime_config(config: &MemoryRuntimeConfig) -> Self {
        Self {
            backend: config.backend,
            profile: config.profile,
            mode: config.mode,
            ingest_mode: config.ingest_mode,
            fail_open: config.fail_open,
            strict_mode_requested: config.strict_mode_requested(),
            strict_mode_active: config.strict_mode_active(),
            effective_fail_open: config.effective_fail_open(),
        }
    }
}

fn registry() -> &'static RwLock<BTreeMap<String, MemorySystemFactory>> {
    MEMORY_SYSTEM_REGISTRY.get_or_init(|| {
        let mut map: BTreeMap<String, MemorySystemFactory> = BTreeMap::new();
        map.insert(
            DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
            Arc::new(|| Box::new(BuiltinMemorySystem)),
        );
        RwLock::new(map)
    })
}

fn normalize_system_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

#[cfg(test)]
fn env_override() -> &'static Mutex<Option<Option<String>>> {
    MEMORY_SYSTEM_ENV_OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub fn register_memory_system<F>(id: &str, factory: F) -> CliResult<()>
where
    F: Fn() -> Box<dyn MemorySystem> + Send + Sync + 'static,
{
    let normalized = normalize_system_id(id);
    if normalized.is_empty() {
        return Err("memory system id must not be empty".to_owned());
    }

    let mut guard = registry()
        .write()
        .map_err(|_error| "memory system registry lock poisoned".to_owned())?;
    guard.insert(normalized, Arc::new(factory));
    Ok(())
}

pub fn list_memory_system_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "memory system registry lock poisoned".to_owned())?;
    Ok(guard.keys().cloned().collect())
}

pub fn list_memory_system_metadata() -> CliResult<Vec<MemorySystemMetadata>> {
    let guard = registry()
        .read()
        .map_err(|_error| "memory system registry lock poisoned".to_owned())?;
    let mut metadata = guard
        .values()
        .map(|factory| factory().metadata())
        .collect::<Vec<_>>();
    metadata.sort_by_key(|entry| entry.id);
    Ok(metadata)
}

pub fn resolve_memory_system(id: Option<&str>) -> CliResult<Box<dyn MemorySystem>> {
    let normalized = id
        .map(normalize_system_id)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MEMORY_SYSTEM_ID.to_owned());

    let guard = registry()
        .read()
        .map_err(|_error| "memory system registry lock poisoned".to_owned())?;
    let Some(factory) = guard.get(&normalized).cloned() else {
        let available = guard.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(format!(
            "memory system `{normalized}` is not registered (available: {available})"
        ));
    };
    Ok(factory())
}

pub fn describe_memory_system(id: Option<&str>) -> CliResult<MemorySystemMetadata> {
    resolve_memory_system(id).map(|system| system.metadata())
}

pub fn memory_system_id_from_env() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(override_value) = env_override().lock().ok().and_then(|guard| guard.clone()) {
            return override_value;
        }
    }

    std::env::var(MEMORY_SYSTEM_ENV)
        .ok()
        .map(|value| normalize_system_id(value.as_str()))
        .filter(|value| !value.is_empty())
}

pub fn resolve_memory_system_selection(config: &LoongClawConfig) -> MemorySystemSelection {
    if let Some(id) = memory_system_id_from_env() {
        return MemorySystemSelection {
            id,
            source: MemorySystemSelectionSource::Env,
        };
    }

    if config.memory.resolved_system() != MemorySystemKind::default() {
        return MemorySystemSelection {
            id: config.memory.resolved_system().as_str().to_owned(),
            source: MemorySystemSelectionSource::Config,
        };
    }

    MemorySystemSelection {
        id: DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
        source: MemorySystemSelectionSource::Default,
    }
}

pub fn collect_memory_system_runtime_snapshot(
    config: &LoongClawConfig,
) -> CliResult<MemorySystemRuntimeSnapshot> {
    let selected = resolve_memory_system_selection(config);
    let selected_metadata = describe_memory_system(Some(selected.id.as_str()))?;
    let available = list_memory_system_metadata()?;
    let runtime = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let policy = MemorySystemPolicySnapshot::from_runtime_config(&runtime);

    Ok(MemorySystemRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        policy,
    })
}

#[cfg(test)]
pub(crate) fn set_memory_system_env_override(value: Option<&str>) {
    let normalized = value
        .map(normalize_system_id)
        .filter(|entry| !entry.is_empty());
    if let Ok(mut guard) = env_override().lock() {
        *guard = Some(normalized);
    }
}

#[cfg(test)]
pub(crate) fn clear_memory_system_env_override() {
    if let Ok(mut guard) = env_override().lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use crate::memory::{MEMORY_SYSTEM_API_VERSION, MemorySystemCapability};

    struct TestRegistrySystem;

    impl MemorySystem for TestRegistrySystem {
        fn id(&self) -> &'static str {
            "registry-test"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-test",
                [MemorySystemCapability::PromptHydration],
                "Test registry system",
            )
        }
    }

    #[test]
    fn resolve_memory_system_includes_builtin() {
        let ids = list_memory_system_ids().expect("list ids");
        assert!(
            ids.iter().any(|id| id == DEFAULT_MEMORY_SYSTEM_ID),
            "builtin memory system should be registered"
        );
    }

    #[test]
    fn registry_can_register_and_resolve_custom_system() {
        register_memory_system("registry-custom", || Box::new(TestRegistrySystem))
            .expect("register custom system");
        let system = resolve_memory_system(Some("registry-custom")).expect("resolve custom system");
        assert_eq!(system.id(), "registry-test");
    }

    #[test]
    fn resolve_memory_system_returns_error_for_unknown_id() {
        let error = match resolve_memory_system(Some("not-registered")) {
            Ok(system) => panic!("expected unknown id to fail, got {}", system.id()),
            Err(error) => error,
        };
        assert!(error.contains("not registered"), "error: {error}");
        assert!(
            error.contains(DEFAULT_MEMORY_SYSTEM_ID),
            "error should include available ids: {error}"
        );
    }

    #[test]
    fn list_memory_system_metadata_exposes_capabilities() {
        let metadata = list_memory_system_metadata().expect("list metadata");
        let builtin = metadata
            .iter()
            .find(|entry| entry.id == DEFAULT_MEMORY_SYSTEM_ID)
            .expect("builtin metadata entry");
        assert_eq!(builtin.api_version, MEMORY_SYSTEM_API_VERSION);
        assert!(
            builtin
                .capabilities
                .contains(&MemorySystemCapability::CanonicalStore),
            "builtin metadata should include canonical-store capability"
        );
    }

    #[test]
    fn memory_system_env_overrides_default_selection() {
        set_memory_system_env_override(Some("builtin"));
        let config = LoongClawConfig::default();
        let selection = resolve_memory_system_selection(&config);
        assert_eq!(selection.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(selection.source, MemorySystemSelectionSource::Env);
        clear_memory_system_env_override();
    }

    #[test]
    fn memory_system_registry_stays_builtin_only_until_adapter_lands() {
        let ids = list_memory_system_ids().expect("list memory-system ids");
        assert!(
            !ids.iter().any(|id| id == "lucid"),
            "future adapter ids should not appear before the adapter actually lands"
        );
    }

    #[test]
    fn memory_system_runtime_snapshot_captures_runtime_policy() {
        let config = LoongClawConfig {
            memory: crate::config::MemoryConfig {
                profile: crate::config::MemoryProfile::WindowPlusSummary,
                fail_open: false,
                ingest_mode: crate::config::MemoryIngestMode::AsyncBackground,
                ..crate::config::MemoryConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.policy.backend, crate::config::MemoryBackendKind::Sqlite);
        assert_eq!(
            snapshot.policy.profile,
            crate::config::MemoryProfile::WindowPlusSummary
        );
        assert_eq!(
            snapshot.policy.mode,
            crate::config::MemoryMode::WindowPlusSummary
        );
        assert_eq!(
            snapshot.policy.ingest_mode,
            crate::config::MemoryIngestMode::AsyncBackground
        );
        assert!(!snapshot.policy.fail_open);
        assert!(snapshot.policy.strict_mode_requested);
        assert!(!snapshot.policy.strict_mode_active);
        assert!(snapshot.policy.effective_fail_open);
    }

    #[test]
    fn memory_system_runtime_snapshot_uses_memory_env_policy_overrides() {
        let mut env = ScopedEnv::new();
        env.set(MEMORY_SYSTEM_ENV, "builtin");
        env.set("LOONGCLAW_MEMORY_PROFILE", "profile_plus_window");
        env.set("LOONGCLAW_MEMORY_FAIL_OPEN", "true");
        env.set("LOONGCLAW_MEMORY_INGEST_MODE", "async_background");

        let config = LoongClawConfig {
            memory: crate::config::MemoryConfig {
                profile: crate::config::MemoryProfile::WindowOnly,
                fail_open: false,
                ingest_mode: crate::config::MemoryIngestMode::SyncMinimal,
                ..crate::config::MemoryConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(snapshot.selected.source, MemorySystemSelectionSource::Env);
        assert_eq!(
            snapshot.policy.profile,
            crate::config::MemoryProfile::ProfilePlusWindow
        );
        assert_eq!(
            snapshot.policy.mode,
            crate::config::MemoryMode::ProfilePlusWindow
        );
        assert!(snapshot.policy.fail_open);
        assert_eq!(
            snapshot.policy.ingest_mode,
            crate::config::MemoryIngestMode::AsyncBackground
        );
    }
}
