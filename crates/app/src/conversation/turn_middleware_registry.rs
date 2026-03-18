use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;

use super::turn_middleware::{ConversationTurnMiddleware, TurnMiddlewareMetadata};

pub const TURN_MIDDLEWARE_ENV: &str = "LOONGCLAW_TURN_MIDDLEWARES";

type TurnMiddlewareFactory = Arc<dyn Fn() -> Box<dyn ConversationTurnMiddleware> + Send + Sync>;

static TURN_MIDDLEWARE_REGISTRY: OnceLock<RwLock<BTreeMap<String, TurnMiddlewareFactory>>> =
    OnceLock::new();
#[cfg(test)]
static TURN_MIDDLEWARE_ENV_OVERRIDE: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();

fn registry() -> &'static RwLock<BTreeMap<String, TurnMiddlewareFactory>> {
    TURN_MIDDLEWARE_REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
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
    guard.insert(normalized, Arc::new(factory));
    Ok(())
}

pub fn list_turn_middleware_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    Ok(guard.keys().cloned().collect())
}

pub fn list_turn_middleware_metadata() -> CliResult<Vec<TurnMiddlewareMetadata>> {
    let guard = registry()
        .read()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    let mut metadata = guard
        .values()
        .map(|factory| factory().metadata())
        .collect::<Vec<_>>();
    metadata.sort_by_key(|entry| entry.id);
    Ok(metadata)
}

pub fn resolve_turn_middleware(id: &str) -> CliResult<Box<dyn ConversationTurnMiddleware>> {
    let normalized = normalize_middleware_id(id);
    if normalized.is_empty() {
        return Err("turn middleware id must not be empty".to_owned());
    }

    let guard = registry()
        .read()
        .map_err(|_error| "turn middleware registry lock poisoned".to_owned())?;
    let Some(factory) = guard.get(&normalized).cloned() else {
        let available = guard.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(format!(
            "turn middleware `{normalized}` is not registered (available: {available})"
        ));
    };
    Ok(factory())
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
mod tests {
    use async_trait::async_trait;

    use crate::config::LoongClawConfig;

    use super::super::context_engine::AssembledConversationContext;
    use super::super::runtime_binding::ConversationRuntimeBinding;
    use super::super::turn_middleware::TurnMiddlewareCapability;
    use super::*;

    struct TestRegistryTurnMiddleware;

    #[async_trait]
    impl ConversationTurnMiddleware for TestRegistryTurnMiddleware {
        fn id(&self) -> &'static str {
            "registry-turn-middleware"
        }

        fn metadata(&self) -> TurnMiddlewareMetadata {
            TurnMiddlewareMetadata::new(self.id(), [TurnMiddlewareCapability::ContextTransform])
        }

        async fn transform_context(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            assembled: AssembledConversationContext,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<AssembledConversationContext> {
            Ok(assembled)
        }
    }

    #[test]
    fn registry_can_register_and_resolve_custom_turn_middleware() {
        register_turn_middleware("registry-turn-middleware", || {
            Box::new(TestRegistryTurnMiddleware)
        })
        .expect("register custom middleware");
        let middleware =
            resolve_turn_middleware("registry-turn-middleware").expect("resolve custom middleware");
        assert_eq!(middleware.id(), "registry-turn-middleware");
    }

    #[test]
    fn resolve_turn_middleware_returns_error_for_unknown_id() {
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
        register_turn_middleware("registry-turn-middleware", || {
            Box::new(TestRegistryTurnMiddleware)
        })
        .expect("register turn middleware");

        let metadata = list_turn_middleware_metadata().expect("list turn middleware metadata");
        let entry = metadata
            .iter()
            .find(|entry| entry.id == "registry-turn-middleware")
            .expect("registry turn middleware metadata");
        assert_eq!(entry.api_version, 1);
        assert!(
            entry
                .capabilities
                .contains(&TurnMiddlewareCapability::ContextTransform)
        );
    }

    #[test]
    fn turn_middleware_ids_from_env_normalizes_and_deduplicates() {
        set_turn_middleware_env_override(Some(" Alpha , beta ,, alpha "));
        let ids = turn_middleware_ids_from_env().expect("turn middleware ids from env");
        assert_eq!(ids, vec!["alpha".to_owned(), "beta".to_owned()]);
        clear_turn_middleware_env_override();
    }
}
