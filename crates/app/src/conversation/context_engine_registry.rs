use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;

use super::context_engine::{
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine, LegacyContextEngine,
};

pub const DEFAULT_CONTEXT_ENGINE_ID: &str = "default";
pub const LEGACY_CONTEXT_ENGINE_ID: &str = "legacy";
pub const CONTEXT_ENGINE_ENV: &str = "LOONGCLAW_CONTEXT_ENGINE";

type ContextEngineFactory = Arc<dyn Fn() -> Box<dyn ConversationContextEngine> + Send + Sync>;

static CONTEXT_ENGINE_REGISTRY: OnceLock<RwLock<BTreeMap<String, ContextEngineFactory>>> =
    OnceLock::new();
#[cfg(test)]
fn context_engine_env_override() -> &'static Mutex<Option<Option<String>>> {
    static OVERRIDE: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

fn registry() -> &'static RwLock<BTreeMap<String, ContextEngineFactory>> {
    CONTEXT_ENGINE_REGISTRY.get_or_init(|| {
        let mut map: BTreeMap<String, ContextEngineFactory> = BTreeMap::new();
        map.insert(
            DEFAULT_CONTEXT_ENGINE_ID.to_owned(),
            Arc::new(|| Box::new(DefaultContextEngine)),
        );
        map.insert(
            LEGACY_CONTEXT_ENGINE_ID.to_owned(),
            Arc::new(|| Box::new(LegacyContextEngine)),
        );
        RwLock::new(map)
    })
}

fn normalize_engine_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

#[cfg(test)]
fn env_override() -> Option<Option<String>> {
    let guard = context_engine_env_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clone()
}

#[cfg(test)]
pub(crate) fn conversation_selector_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn register_context_engine<F>(id: &str, factory: F) -> CliResult<()>
where
    F: Fn() -> Box<dyn ConversationContextEngine> + Send + Sync + 'static,
{
    let normalized = normalize_engine_id(id);
    if normalized.is_empty() {
        return Err("context engine id must not be empty".to_owned());
    }

    let mut guard = registry()
        .write()
        .map_err(|_error| "context engine registry lock poisoned".to_owned())?;
    guard.insert(normalized, Arc::new(factory));
    Ok(())
}

pub fn list_context_engine_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "context engine registry lock poisoned".to_owned())?;
    Ok(guard.keys().cloned().collect())
}

pub fn list_context_engine_metadata() -> CliResult<Vec<ContextEngineMetadata>> {
    let guard = registry()
        .read()
        .map_err(|_error| "context engine registry lock poisoned".to_owned())?;
    let mut metadata = guard
        .values()
        .map(|factory| factory().metadata())
        .collect::<Vec<_>>();
    metadata.sort_by_key(|entry| entry.id);
    Ok(metadata)
}

pub fn resolve_context_engine(id: Option<&str>) -> CliResult<Box<dyn ConversationContextEngine>> {
    let normalized = id
        .map(normalize_engine_id)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CONTEXT_ENGINE_ID.to_owned());

    let guard = registry()
        .read()
        .map_err(|_error| "context engine registry lock poisoned".to_owned())?;
    let Some(factory) = guard.get(&normalized).cloned() else {
        let available = guard.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(format!(
            "context engine `{normalized}` is not registered (available: {available})"
        ));
    };
    Ok(factory())
}

pub fn describe_context_engine(id: Option<&str>) -> CliResult<ContextEngineMetadata> {
    resolve_context_engine(id).map(|engine| engine.metadata())
}

pub fn context_engine_id_from_env() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(override_value) = env_override() {
            return override_value;
        }
    }

    std::env::var(CONTEXT_ENGINE_ENV)
        .ok()
        .map(|value| normalize_engine_id(value.as_str()))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
pub(crate) fn set_context_engine_env_override(value: Option<&str>) {
    let normalized = value
        .map(normalize_engine_id)
        .filter(|entry| !entry.is_empty());
    let mut guard = context_engine_env_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(normalized);
}

#[cfg(test)]
pub(crate) fn clear_context_engine_env_override() {
    let mut guard = context_engine_env_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

#[cfg(test)]
struct ScopedContextEngineEnvOverride {
    previous: Option<Option<String>>,
}

#[cfg(test)]
impl ScopedContextEngineEnvOverride {
    fn set(value: Option<&str>) -> Self {
        let previous = env_override();
        set_context_engine_env_override(value);
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for ScopedContextEngineEnvOverride {
    fn drop(&mut self) {
        let mut guard = context_engine_env_override()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = self.previous.clone();
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::Value;

    use super::super::runtime_binding::ConversationRuntimeBinding;
    use crate::config::LoongClawConfig;

    use super::super::context_engine::ContextEngineCapability;
    use super::*;

    struct TestRegistryEngine;

    #[async_trait]
    impl ConversationContextEngine for TestRegistryEngine {
        fn id(&self) -> &'static str {
            "registry-test"
        }

        async fn assemble_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn resolve_context_engine_includes_default() {
        let ids = list_context_engine_ids().expect("list ids");
        assert!(
            ids.iter().any(|id| id == DEFAULT_CONTEXT_ENGINE_ID),
            "default context engine should be registered"
        );
        assert!(
            ids.iter().any(|id| id == LEGACY_CONTEXT_ENGINE_ID),
            "legacy context engine should be registered"
        );
    }

    #[test]
    fn registry_can_register_and_resolve_custom_engine() {
        register_context_engine("registry-custom", || Box::new(TestRegistryEngine))
            .expect("register custom engine");
        let engine = resolve_context_engine(Some("registry-custom")).expect("resolve custom");
        assert_eq!(engine.id(), "registry-test");
    }

    #[test]
    fn resolve_context_engine_returns_error_for_unknown_id() {
        let error = match resolve_context_engine(Some("not-registered")) {
            Ok(engine) => panic!("expected unknown id to fail, got {}", engine.id()),
            Err(error) => error,
        };
        assert!(error.contains("not registered"), "error: {error}");
        assert!(
            error.contains(DEFAULT_CONTEXT_ENGINE_ID),
            "error should include available ids: {error}"
        );
    }

    #[test]
    fn list_context_engine_metadata_exposes_capabilities() {
        let metadata = list_context_engine_metadata().expect("list metadata");

        let default = metadata
            .iter()
            .find(|entry| entry.id == DEFAULT_CONTEXT_ENGINE_ID)
            .expect("default metadata entry");
        assert_eq!(default.api_version, 1);

        let legacy = metadata
            .iter()
            .find(|entry| entry.id == LEGACY_CONTEXT_ENGINE_ID)
            .expect("legacy metadata entry");
        assert!(
            legacy
                .capabilities
                .contains(&ContextEngineCapability::LegacyMessageAssembly),
            "legacy metadata should include legacy assembly capability"
        );
    }

    #[test]
    fn describe_context_engine_uses_default_when_id_absent() {
        let metadata = describe_context_engine(None).expect("describe default engine");
        assert_eq!(metadata.id, DEFAULT_CONTEXT_ENGINE_ID);
    }

    #[test]
    fn context_engine_env_override_is_visible_across_threads() {
        let _env_lock = conversation_selector_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _scoped_env = ScopedContextEngineEnvOverride::set(Some("registry-custom"));

        let observed = std::thread::spawn(context_engine_id_from_env)
            .join()
            .expect("join thread");

        assert_eq!(observed.as_deref(), Some("registry-custom"));
    }

    #[test]
    fn scoped_context_engine_env_override_clears_on_panic() {
        let _env_lock = conversation_selector_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let result = std::panic::catch_unwind(|| {
            let _scoped_env = ScopedContextEngineEnvOverride::set(Some("registry-custom"));
            assert_eq!(
                context_engine_id_from_env().as_deref(),
                Some("registry-custom")
            );
            panic!("boom");
        });

        assert!(result.is_err(), "panic path should be captured");
        assert_eq!(
            context_engine_id_from_env(),
            None,
            "panic cleanup should restore the previous override state"
        );
    }
}
