use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;

use super::turn_middleware::{
    BUILTIN_TURN_MIDDLEWARES, ConversationTurnMiddleware, TurnMiddlewareMetadata,
};

pub const TURN_MIDDLEWARE_ENV: &str = "LOONGCLAW_TURN_MIDDLEWARES";

type TurnMiddlewareFactory = Arc<dyn Fn() -> Box<dyn ConversationTurnMiddleware> + Send + Sync>;

#[derive(Clone)]
struct TurnMiddlewareRegistration {
    factory: TurnMiddlewareFactory,
    default_enabled: bool,
}

impl TurnMiddlewareRegistration {
    fn builtin(factory: TurnMiddlewareFactory) -> Self {
        Self {
            factory,
            default_enabled: true,
        }
    }

    fn custom(factory: TurnMiddlewareFactory) -> Self {
        Self {
            factory,
            default_enabled: false,
        }
    }
}

static TURN_MIDDLEWARE_REGISTRY: OnceLock<RwLock<BTreeMap<String, TurnMiddlewareRegistration>>> =
    OnceLock::new();
#[cfg(test)]
static TURN_MIDDLEWARE_ENV_OVERRIDE: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();

fn registry() -> &'static RwLock<BTreeMap<String, TurnMiddlewareRegistration>> {
    TURN_MIDDLEWARE_REGISTRY.get_or_init(|| {
        let mut map: BTreeMap<String, TurnMiddlewareRegistration> = BTreeMap::new();
        for spec in BUILTIN_TURN_MIDDLEWARES {
            let factory: TurnMiddlewareFactory = Arc::new(spec.factory);
            map.insert(
                spec.id.to_owned(),
                TurnMiddlewareRegistration::builtin(factory),
            );
        }
        RwLock::new(map)
    })
}

fn normalize_middleware_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn normalize_middleware_ids<'a, I>(raw_ids: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();

    for raw in raw_ids {
        let normalized = normalize_middleware_id(raw);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        ids.push(normalized);
    }

    ids
}

#[cfg(test)]
fn env_override() -> &'static Mutex<Option<Option<String>>> {
    TURN_MIDDLEWARE_ENV_OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub fn register_turn_middleware<F>(id: &str, factory: F) -> CliResult<()>
where
    F: Fn() -> Box<dyn ConversationTurnMiddleware> + Send + Sync + 'static,
{
    let normalized = normalize_middleware_id(id);
    if normalized.is_empty() {
        return Err("turn middleware id must not be empty".to_owned());
    }

    {
        let guard = registry()
            .read()
            .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
        if guard.contains_key(&normalized) {
            return Err(format!(
                "turn middleware `{normalized}` is already registered"
            ));
        }
    }

    let middleware = factory();
    let middleware_id = normalize_middleware_id(middleware.id());
    let metadata_id = normalize_middleware_id(middleware.metadata().id);
    if normalized != middleware_id || normalized != metadata_id {
        return Err(format!(
            "registered turn middleware id `{normalized}` must match middleware.id `{}` and metadata.id `{}`",
            middleware.id(),
            middleware.metadata().id
        ));
    }

    let mut guard = registry()
        .write()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    if guard.contains_key(&normalized) {
        return Err(format!(
            "turn middleware `{normalized}` is already registered"
        ));
    }
    guard.insert(
        normalized,
        TurnMiddlewareRegistration::custom(Arc::new(factory)),
    );
    Ok(())
}

pub fn list_turn_middleware_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    Ok(guard.keys().cloned().collect())
}

pub fn list_turn_middleware_metadata() -> CliResult<Vec<TurnMiddlewareMetadata>> {
    let factories = {
        let guard = registry()
            .read()
            .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
        guard
            .values()
            .map(|registration| registration.factory.clone())
            .collect::<Vec<_>>()
    };
    let mut metadata = factories
        .into_iter()
        .map(|factory| factory().metadata())
        .collect::<Vec<_>>();
    metadata.sort_by_key(|entry| entry.id);
    Ok(metadata)
}

pub fn default_turn_middleware_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    Ok(guard
        .iter()
        .filter_map(|(id, registration)| registration.default_enabled.then_some(id.clone()))
        .collect())
}

pub fn resolve_turn_middleware(id: &str) -> CliResult<Box<dyn ConversationTurnMiddleware>> {
    let normalized = normalize_middleware_id(id);
    if normalized.is_empty() {
        return Err("turn middleware id must not be empty".to_owned());
    }

    let registration = {
        let guard = registry()
            .read()
            .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
        let Some(registration) = guard.get(&normalized).cloned() else {
            let available = guard.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(format!(
                "turn middleware `{normalized}` is not registered (available: {available})"
            ));
        };
        registration
    };
    Ok((registration.factory)())
}

pub fn resolve_turn_middlewares(
    ids: &[String],
) -> CliResult<Vec<Box<dyn ConversationTurnMiddleware>>> {
    ids.iter()
        .map(|id| resolve_turn_middleware(id.as_str()))
        .collect()
}

pub fn describe_turn_middlewares(ids: &[String]) -> CliResult<Vec<TurnMiddlewareMetadata>> {
    ids.iter()
        .map(|id| resolve_turn_middleware(id.as_str()).map(|middleware| middleware.metadata()))
        .collect()
}

pub fn turn_middleware_ids_from_env() -> Option<Vec<String>> {
    #[cfg(test)]
    {
        if let Some(override_value) = env_override().lock().ok().and_then(|guard| guard.clone()) {
            return override_value.and_then(|raw| {
                let normalized = normalize_middleware_ids(raw.split(','));
                (!normalized.is_empty()).then_some(normalized)
            });
        }
    }

    std::env::var(TURN_MIDDLEWARE_ENV).ok().and_then(|value| {
        let normalized = normalize_middleware_ids(value.split(','));
        (!normalized.is_empty()).then_some(normalized)
    })
}

#[cfg(test)]
pub(crate) fn set_turn_middleware_env_override(value: Option<&str>) {
    if let Ok(mut guard) = env_override().lock() {
        *guard = Some(value.map(str::to_owned));
    }
}

#[cfg(test)]
pub(crate) fn clear_turn_middleware_env_override() {
    if let Ok(mut guard) = env_override().lock() {
        *guard = None;
    }
}

#[cfg(test)]
struct ScopedTurnMiddlewareEnvOverride {
    previous: Option<Option<String>>,
}

#[cfg(test)]
impl ScopedTurnMiddlewareEnvOverride {
    fn set(value: Option<&str>) -> Self {
        let mut guard = env_override()
            .lock()
            .expect("turn middleware env override lock");
        let previous = guard.clone();
        *guard = Some(value.map(str::to_owned));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for ScopedTurnMiddlewareEnvOverride {
    fn drop(&mut self) {
        if let Ok(mut guard) = env_override().lock() {
            *guard = self.previous.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;

    use super::super::turn_middleware::{
        SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID, SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID,
    };
    use super::*;

    fn registry_test_guard() -> MutexGuard<'static, ()> {
        super::super::context_engine_registry::conversation_selector_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct StaticIdTurnMiddleware {
        id: &'static str,
    }

    #[async_trait]
    impl ConversationTurnMiddleware for StaticIdTurnMiddleware {
        fn id(&self) -> &'static str {
            self.id
        }
    }

    #[test]
    fn list_turn_middleware_ids_includes_builtin_defaults() {
        let _registry_lock = registry_test_guard();
        let ids = list_turn_middleware_ids().expect("list ids");
        assert!(
            ids.iter()
                .any(|id| id == SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID),
            "system prompt addition middleware should be registered by default"
        );
        assert!(
            ids.iter()
                .any(|id| id == SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID),
            "system prompt tool-view middleware should be registered by default"
        );
    }

    #[test]
    fn registry_can_register_and_resolve_custom_turn_middleware() {
        let _registry_lock = registry_test_guard();
        register_turn_middleware("registry-turn-middleware-custom", || {
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-custom",
            })
        })
        .expect("register custom middleware");
        let middleware = resolve_turn_middleware("registry-turn-middleware-custom")
            .expect("resolve custom middleware");
        assert_eq!(middleware.id(), "registry-turn-middleware-custom");
    }

    #[test]
    fn resolve_turn_middleware_returns_error_for_unknown_id() {
        let _registry_lock = registry_test_guard();
        let error = match resolve_turn_middleware("not-registered") {
            Ok(middleware) => panic!(
                "expected unknown turn middleware to fail, got {}",
                middleware.id()
            ),
            Err(error) => error,
        };
        assert!(error.contains("not registered"), "error: {error}");
    }

    #[test]
    fn list_turn_middleware_metadata_exposes_capabilities() {
        let _registry_lock = registry_test_guard();
        register_turn_middleware("registry-turn-middleware-capability", || {
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-capability",
            })
        })
        .expect("register turn middleware");

        let metadata = list_turn_middleware_metadata().expect("list turn middleware metadata");
        let entry = metadata
            .iter()
            .find(|entry| entry.id == "registry-turn-middleware-capability")
            .expect("registry turn middleware metadata");
        assert_eq!(entry.api_version, 1);
        assert!(entry.capabilities.is_empty());
    }

    #[test]
    fn register_turn_middleware_rejects_duplicate_id() {
        let _registry_lock = registry_test_guard();
        register_turn_middleware("registry-turn-middleware-duplicate", || {
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-duplicate",
            })
        })
        .expect("register duplicate test middleware");

        let error = register_turn_middleware("registry-turn-middleware-duplicate", || {
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-duplicate",
            })
        })
        .expect_err("duplicate registration should fail");
        assert!(
            error.contains("already registered"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn register_turn_middleware_duplicate_id_does_not_invoke_factory() {
        let _registry_lock = registry_test_guard();
        register_turn_middleware("registry-turn-middleware-duplicate-fast-fail", || {
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-duplicate-fast-fail",
            })
        })
        .expect("register duplicate fast-fail test middleware");

        let duplicate_factory_calls = Arc::new(AtomicUsize::new(0));
        let observed = duplicate_factory_calls.clone();
        let error =
            register_turn_middleware("registry-turn-middleware-duplicate-fast-fail", move || {
                observed.fetch_add(1, Ordering::SeqCst);
                Box::new(StaticIdTurnMiddleware {
                    id: "registry-turn-middleware-duplicate-fast-fail",
                })
            })
            .expect_err("duplicate registration should fast-fail before invoking the factory");
        assert!(
            error.contains("already registered"),
            "unexpected error: {error}"
        );
        assert_eq!(
            duplicate_factory_calls.load(Ordering::SeqCst),
            0,
            "duplicate registration should not invoke the replacement factory"
        );
    }

    #[test]
    fn list_turn_middleware_metadata_runs_factories_outside_registry_lock() {
        let _registry_lock = registry_test_guard();
        let factory_ran_outside_registry_lock = Arc::new(AtomicBool::new(false));
        let observed = factory_ran_outside_registry_lock.clone();
        register_turn_middleware("registry-turn-middleware-metadata-lock-scope", move || {
            observed.store(registry().try_write().is_ok(), Ordering::SeqCst);
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-metadata-lock-scope",
            })
        })
        .expect("register turn middleware");

        let metadata = list_turn_middleware_metadata().expect("list turn middleware metadata");
        assert!(
            metadata
                .iter()
                .any(|entry| entry.id == "registry-turn-middleware-metadata-lock-scope")
        );
        assert!(
            factory_ran_outside_registry_lock.load(Ordering::SeqCst),
            "metadata factory should not run while the registry read lock is held"
        );
    }

    #[test]
    fn resolve_turn_middleware_runs_factory_outside_registry_lock() {
        let _registry_lock = registry_test_guard();
        let factory_ran_outside_registry_lock = Arc::new(AtomicBool::new(false));
        let observed = factory_ran_outside_registry_lock.clone();
        register_turn_middleware("registry-turn-middleware-resolve-lock-scope", move || {
            observed.store(registry().try_write().is_ok(), Ordering::SeqCst);
            Box::new(StaticIdTurnMiddleware {
                id: "registry-turn-middleware-resolve-lock-scope",
            })
        })
        .expect("register turn middleware");

        let middleware = resolve_turn_middleware("registry-turn-middleware-resolve-lock-scope")
            .expect("resolve turn middleware");
        assert_eq!(
            middleware.id(),
            "registry-turn-middleware-resolve-lock-scope"
        );
        assert!(
            factory_ran_outside_registry_lock.load(Ordering::SeqCst),
            "middleware factory should not run while the registry read lock is held"
        );
    }

    #[test]
    fn turn_middleware_ids_from_env_normalizes_and_deduplicates() {
        let _registry_lock = registry_test_guard();
        let _scoped_env = ScopedTurnMiddlewareEnvOverride::set(Some(" Alpha , beta ,, alpha "));
        let ids = turn_middleware_ids_from_env().expect("turn middleware ids from env");
        assert_eq!(ids, vec!["alpha".to_owned(), "beta".to_owned()]);
    }
}
