use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;
use crate::config::{
    LoongClawConfig, MemoryBackendKind, MemoryIngestMode, MemoryMode, MemoryProfile,
    MemorySystemKind,
};

use super::runtime_config::MemoryRuntimeConfig;
use super::system::{
    BuiltinMemorySystem, DEFAULT_MEMORY_SYSTEM_ID, MemorySystem, MemorySystemMetadata,
    RECALL_FIRST_MEMORY_SYSTEM_ID, RecallFirstMemorySystem, WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
    WorkspaceRecallMemorySystem,
};

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
        map.insert(
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned(),
            Arc::new(|| Box::new(WorkspaceRecallMemorySystem)),
        );
        map.insert(
            RECALL_FIRST_MEMORY_SYSTEM_ID.to_owned(),
            Arc::new(|| Box::new(RecallFirstMemorySystem)),
        );
        RwLock::new(map)
    })
}

#[cfg(test)]
fn env_override() -> &'static Mutex<Option<Option<String>>> {
    MEMORY_SYSTEM_ENV_OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub fn register_memory_system<F>(id: &str, factory: F) -> CliResult<()>
where
    F: Fn() -> Box<dyn MemorySystem> + Send + Sync + 'static,
{
    let normalized = super::normalize_system_id(id)
        .ok_or_else(|| "memory system id must not be empty".to_owned())?;
    let reserved_system_id = match normalized.as_str() {
        DEFAULT_MEMORY_SYSTEM_ID => Some(DEFAULT_MEMORY_SYSTEM_ID),
        WORKSPACE_RECALL_MEMORY_SYSTEM_ID => Some(WORKSPACE_RECALL_MEMORY_SYSTEM_ID),
        RECALL_FIRST_MEMORY_SYSTEM_ID => Some(RECALL_FIRST_MEMORY_SYSTEM_ID),
        _ => None,
    };
    if let Some(reserved_system_id) = reserved_system_id {
        return Err(format!(
            "memory system `{reserved_system_id}` is reserved and cannot be overridden"
        ));
    }

    let system = factory();
    let runtime_id = super::normalize_system_id(system.id())
        .ok_or_else(|| "memory system runtime id must not be empty".to_owned())?;
    let metadata = system.metadata();
    let metadata_id = super::normalize_system_id(metadata.id)
        .ok_or_else(|| "memory system metadata id must not be empty".to_owned())?;
    if runtime_id != normalized || metadata_id != normalized {
        return Err(format!(
            "registered memory system id `{normalized}` must match system.id `{}` and metadata.id `{}`",
            system.id(),
            metadata.id
        ));
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
        .and_then(super::normalize_system_id)
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
        .and_then(|value| super::normalize_system_id(value.as_str()))
}

pub(crate) fn registered_memory_system_id(id: Option<&str>) -> Option<String> {
    let normalized = id.and_then(super::normalize_system_id)?;
    let guard = registry().read().ok()?;
    guard.contains_key(&normalized).then_some(normalized)
}

pub(crate) fn registered_memory_system_id_from_env() -> Option<String> {
    let env_id = memory_system_id_from_env();
    registered_memory_system_id(env_id.as_deref())
}

pub fn supported_memory_system_kind_from_env() -> Option<MemorySystemKind> {
    registered_memory_system_id_from_env()
        .as_deref()
        .and_then(MemorySystemKind::parse_id)
}

pub fn resolve_memory_system_selection(config: &LoongClawConfig) -> MemorySystemSelection {
    if let Some(system_id) = registered_memory_system_id_from_env() {
        return MemorySystemSelection {
            id: system_id,
            source: MemorySystemSelectionSource::Env,
        };
    }

    if let Some(config_system_id) = config.memory.system_id.as_deref() {
        if let Some(system_id) = registered_memory_system_id(Some(config_system_id)) {
            return MemorySystemSelection {
                id: system_id,
                source: MemorySystemSelectionSource::Config,
            };
        }
    } else if config.memory.resolved_system() != MemorySystemKind::default() {
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
mod tests {
    use super::*;
    use crate::memory::{MEMORY_SYSTEM_API_VERSION, MemoryRecallMode, MemorySystemCapability};
    use crate::test_support::ScopedEnv;

    fn clear_memory_runtime_env_overrides(env: &mut ScopedEnv) {
        env.remove(MEMORY_SYSTEM_ENV);
        env.remove("LOONGCLAW_MEMORY_BACKEND");
        env.remove("LOONGCLAW_MEMORY_PROFILE");
        env.remove("LOONGCLAW_MEMORY_FAIL_OPEN");
        env.remove("LOONGCLAW_MEMORY_INGEST_MODE");
        env.remove("LOONGCLAW_SQLITE_PATH");
        env.remove("LOONGCLAW_SLIDING_WINDOW");
        env.remove("LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS");
        env.remove("LOONGCLAW_MEMORY_PROFILE_NOTE");
    }

    struct MatchingRegistrySystem;

    impl MemorySystem for MatchingRegistrySystem {
        fn id(&self) -> &'static str {
            "registry-custom"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-custom",
                [MemorySystemCapability::PromptHydration],
                "Test registry system",
            )
        }
    }

    struct MismatchedRegistrySystem;

    impl MemorySystem for MismatchedRegistrySystem {
        fn id(&self) -> &'static str {
            "registry-mismatch"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-mismatch",
                [MemorySystemCapability::PromptHydration],
                "Mismatched registry system",
            )
        }
    }

    struct StageAwareRegistrySystem;

    impl MemorySystem for StageAwareRegistrySystem {
        fn id(&self) -> &'static str {
            "registry-stage-aware-snapshot"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-stage-aware-snapshot",
                [MemorySystemCapability::PromptHydration],
                "Registry snapshot system",
            )
            .with_supported_pre_assembly_stage_families([
                crate::memory::MemoryStageFamily::Retrieve,
            ])
        }
    }

    struct StageAwareRegistrySystemForConfigSelection;

    impl MemorySystem for StageAwareRegistrySystemForConfigSelection {
        fn id(&self) -> &'static str {
            "registry-stage-aware-snapshot-config"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-stage-aware-snapshot-config",
                [MemorySystemCapability::PromptHydration],
                "Registry config snapshot system",
            )
            .with_supported_pre_assembly_stage_families([
                crate::memory::MemoryStageFamily::Retrieve,
            ])
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
        register_memory_system("registry-custom", || Box::new(MatchingRegistrySystem))
            .expect("register custom system");
        let system = resolve_memory_system(Some("registry-custom")).expect("resolve custom system");
        assert_eq!(system.id(), "registry-custom");
    }

    #[test]
    fn registry_rejects_builtin_override() {
        let error = register_memory_system(DEFAULT_MEMORY_SYSTEM_ID, || {
            Box::new(MatchingRegistrySystem)
        })
        .expect_err("builtin memory system should stay reserved");
        assert!(error.contains("reserved"), "error: {error}");
    }

    #[test]
    fn registry_rejects_workspace_recall_override() {
        let error = register_memory_system(WORKSPACE_RECALL_MEMORY_SYSTEM_ID, || {
            Box::new(MatchingRegistrySystem)
        })
        .expect_err("workspace_recall memory system should stay reserved");
        assert!(error.contains("reserved"), "error: {error}");
    }

    #[test]
    fn registry_rejects_recall_first_override() {
        let error = register_memory_system(RECALL_FIRST_MEMORY_SYSTEM_ID, || {
            Box::new(MatchingRegistrySystem)
        })
        .expect_err("recall_first memory system should stay reserved");
        assert!(error.contains("reserved"), "error: {error}");
    }

    #[test]
    fn registry_rejects_registry_id_mismatches() {
        let error = register_memory_system("registry-custom-alias", || {
            Box::new(MismatchedRegistrySystem)
        })
        .expect_err("registry id mismatch should fail");
        assert!(error.contains("must match"), "error: {error}");
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
        assert_eq!(
            builtin.supported_recall_modes,
            vec![MemoryRecallMode::PromptAssembly]
        );

        let workspace_recall = metadata
            .iter()
            .find(|entry| entry.id == WORKSPACE_RECALL_MEMORY_SYSTEM_ID)
            .expect("workspace_recall metadata entry");
        assert!(
            workspace_recall
                .capabilities
                .contains(&MemorySystemCapability::RetrievalProvenance),
            "workspace_recall metadata should include retrieval provenance capability"
        );
        assert_eq!(
            workspace_recall.supported_recall_modes,
            vec![MemoryRecallMode::PromptAssembly]
        );

        let recall_first = metadata
            .iter()
            .find(|entry| entry.id == RECALL_FIRST_MEMORY_SYSTEM_ID)
            .expect("recall_first metadata entry");
        assert!(
            recall_first
                .capabilities
                .contains(&MemorySystemCapability::RetrievalProvenance),
            "recall_first metadata should include retrieval provenance capability"
        );
    }

    #[test]
    fn memory_system_env_overrides_default_selection() {
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);
        env.set(MEMORY_SYSTEM_ENV, "builtin");
        let config = LoongClawConfig::default();
        let selection = resolve_memory_system_selection(&config);
        assert_eq!(selection.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(selection.source, MemorySystemSelectionSource::Env);
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
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);

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

        assert_eq!(
            snapshot.policy.backend,
            crate::config::MemoryBackendKind::Sqlite
        );
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
        clear_memory_runtime_env_overrides(&mut env);
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

    #[test]
    fn registry_backed_memory_system_env_surfaces_in_runtime_snapshot() {
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);
        register_memory_system("registry-custom", || Box::new(MatchingRegistrySystem))
            .expect("register custom registry system");
        env.set(MEMORY_SYSTEM_ENV, "registry-custom");

        let config = LoongClawConfig::default();
        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, "registry-custom");
        assert_eq!(snapshot.selected.source, MemorySystemSelectionSource::Env);
    }

    #[test]
    fn invalid_memory_system_env_is_ignored_so_snapshot_matches_runtime_behavior() {
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);
        env.set(MEMORY_SYSTEM_ENV, "lucid");

        let config = LoongClawConfig::default();
        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(
            snapshot.selected.source,
            MemorySystemSelectionSource::Default
        );
    }

    #[test]
    fn memory_system_field_surfaces_registry_backed_selection_and_stage_metadata_in_snapshot() {
        register_memory_system("registry-stage-aware-snapshot", || {
            Box::new(StageAwareRegistrySystem)
        })
        .expect("register stage-aware registry system");

        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);
        env.set(MEMORY_SYSTEM_ENV, "registry-stage-aware-snapshot");

        let config = LoongClawConfig::default();
        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, "registry-stage-aware-snapshot");
        assert_eq!(snapshot.selected.source, MemorySystemSelectionSource::Env);
        assert_eq!(
            snapshot.selected_metadata.id,
            "registry-stage-aware-snapshot"
        );
        assert_eq!(
            snapshot
                .selected_metadata
                .supported_pre_assembly_stage_families,
            vec![crate::memory::MemoryStageFamily::Retrieve]
        );
    }

    #[test]
    fn registry_backed_memory_system_config_selection_surfaces_in_runtime_snapshot() {
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);
        register_memory_system("registry-stage-aware-snapshot-config", || {
            Box::new(StageAwareRegistrySystemForConfigSelection)
        })
        .expect("register config-selected registry system");

        let config = LoongClawConfig {
            memory: crate::config::MemoryConfig {
                system_id: Some("registry-stage-aware-snapshot-config".to_owned()),
                ..crate::config::MemoryConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, "registry-stage-aware-snapshot-config");
        assert_eq!(
            snapshot.selected.source,
            MemorySystemSelectionSource::Config
        );
        assert_eq!(
            snapshot.selected_metadata.id,
            "registry-stage-aware-snapshot-config"
        );
        assert_eq!(
            snapshot
                .selected_metadata
                .supported_pre_assembly_stage_families,
            vec![crate::memory::MemoryStageFamily::Retrieve]
        );
    }

    #[test]
    fn unknown_config_selected_memory_system_falls_back_to_builtin_snapshot() {
        let mut env = ScopedEnv::new();
        clear_memory_runtime_env_overrides(&mut env);

        let config = LoongClawConfig {
            memory: crate::config::MemoryConfig {
                system_id: Some("lucid".to_owned()),
                ..crate::config::MemoryConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let snapshot =
            collect_memory_system_runtime_snapshot(&config).expect("collect runtime snapshot");

        assert_eq!(snapshot.selected.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(
            snapshot.selected.source,
            MemorySystemSelectionSource::Default
        );
    }
}
