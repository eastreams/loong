use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock, RwLock};

use crate::CliResult;

use super::acpx::AcpxCliProbeBackend;
use super::backend::{AcpBackendMetadata, AcpRuntimeBackend, PlanningStubAcpBackend};

pub const DEFAULT_ACP_BACKEND_ID: &str = "planning_stub";
pub const ACP_BACKEND_ENV: &str = "LOONGCLAW_ACP_BACKEND";

type SharedAcpBackend = Arc<dyn AcpRuntimeBackend>;

static ACP_BACKEND_REGISTRY: OnceLock<RwLock<BTreeMap<String, SharedAcpBackend>>> = OnceLock::new();
#[cfg(test)]
static ACP_BACKEND_ENV_OVERRIDE: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();

fn registry() -> &'static RwLock<BTreeMap<String, SharedAcpBackend>> {
    ACP_BACKEND_REGISTRY.get_or_init(|| {
        let mut map: BTreeMap<String, SharedAcpBackend> = BTreeMap::new();
        map.insert(
            DEFAULT_ACP_BACKEND_ID.to_owned(),
            Arc::new(PlanningStubAcpBackend),
        );
        map.insert("acpx".to_owned(), Arc::new(AcpxCliProbeBackend));
        RwLock::new(map)
    })
}

fn normalize_backend_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

#[cfg(test)]
fn env_override() -> &'static Mutex<Option<Option<String>>> {
    ACP_BACKEND_ENV_OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub fn register_acp_backend<F>(id: &str, factory: F) -> CliResult<()>
where
    F: Fn() -> Box<dyn AcpRuntimeBackend> + Send + Sync + 'static,
{
    let normalized = normalize_backend_id(id);
    if normalized.is_empty() {
        return Err("ACP backend id must not be empty".to_owned());
    }

    let mut guard = registry()
        .write()
        .map_err(|_error| "ACP backend registry lock poisoned".to_owned())?;
    guard.insert(normalized, Arc::<dyn AcpRuntimeBackend>::from(factory()));
    Ok(())
}

pub fn list_acp_backend_ids() -> CliResult<Vec<String>> {
    let guard = registry()
        .read()
        .map_err(|_error| "ACP backend registry lock poisoned".to_owned())?;
    Ok(guard.keys().cloned().collect())
}

pub fn list_acp_backend_metadata() -> CliResult<Vec<AcpBackendMetadata>> {
    let guard = registry()
        .read()
        .map_err(|_error| "ACP backend registry lock poisoned".to_owned())?;
    let mut metadata = guard
        .values()
        .map(|backend| backend.metadata())
        .collect::<Vec<_>>();
    metadata.sort_by_key(|entry| entry.id);
    Ok(metadata)
}

pub fn resolve_acp_backend(id: Option<&str>) -> CliResult<Arc<dyn AcpRuntimeBackend>> {
    let normalized = id
        .map(normalize_backend_id)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_ACP_BACKEND_ID.to_owned());

    let guard = registry()
        .read()
        .map_err(|_error| "ACP backend registry lock poisoned".to_owned())?;
    let Some(factory) = guard.get(&normalized).cloned() else {
        let available = guard.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(format!(
            "ACP backend `{normalized}` is not registered (available: {available})"
        ));
    };
    Ok(factory)
}

pub fn describe_acp_backend(id: Option<&str>) -> CliResult<AcpBackendMetadata> {
    resolve_acp_backend(id).map(|backend| backend.metadata())
}

pub fn acp_backend_id_from_env() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(override_value) = env_override().lock().ok().and_then(|guard| guard.clone()) {
            return override_value;
        }
    }

    std::env::var(ACP_BACKEND_ENV)
        .ok()
        .map(|value| normalize_backend_id(value.as_str()))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
pub(crate) fn set_acp_backend_env_override(value: Option<&str>) {
    let normalized = value
        .map(normalize_backend_id)
        .filter(|entry| !entry.is_empty());
    if let Ok(mut guard) = env_override().lock() {
        *guard = Some(normalized);
    }
}

#[cfg(test)]
pub(crate) fn clear_acp_backend_env_override() {
    if let Ok(mut guard) = env_override().lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::super::backend::{
        AcpCapability, AcpSessionBootstrap, AcpSessionHandle, AcpSessionMode, AcpSessionState,
        AcpTurnRequest, AcpTurnResult, AcpTurnStopReason,
    };
    use super::*;
    use crate::config::LoongClawConfig;

    struct TestAcpBackend;

    #[async_trait]
    impl AcpRuntimeBackend for TestAcpBackend {
        fn id(&self) -> &'static str {
            "registry_test"
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [AcpCapability::TurnExecution],
                "Registry test backend",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: "registry-test-runtime".to_owned(),
                working_directory: request.working_directory.clone(),
                backend_session_id: None,
                agent_session_id: None,
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn set_mode(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            _mode: AcpSessionMode,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[test]
    fn resolve_acp_backend_includes_default() {
        let ids = list_acp_backend_ids().expect("list ACP backend ids");
        assert!(
            ids.iter().any(|id| id == DEFAULT_ACP_BACKEND_ID),
            "default ACP backend should be registered"
        );
        assert!(
            ids.iter().any(|id| id == "acpx"),
            "acpx backend scaffold should be registered"
        );
    }

    #[test]
    fn registry_can_register_and_resolve_custom_backend() {
        register_acp_backend("registry-custom", || Box::new(TestAcpBackend))
            .expect("register custom ACP backend");
        let backend = resolve_acp_backend(Some("registry-custom")).expect("resolve custom backend");
        assert_eq!(backend.id(), "registry_test");
    }

    #[test]
    fn resolve_acp_backend_returns_error_for_unknown_id() {
        let error = match resolve_acp_backend(Some("not-registered")) {
            Ok(backend) => panic!("expected unknown id to fail, got {}", backend.id()),
            Err(error) => error,
        };
        assert!(error.contains("not registered"), "error: {error}");
        assert!(
            error.contains(DEFAULT_ACP_BACKEND_ID),
            "error should include available ids: {error}"
        );
    }

    #[test]
    fn list_acp_backend_metadata_exposes_capabilities() {
        register_acp_backend("registry-capability", || Box::new(TestAcpBackend))
            .expect("register capability backend");
        let metadata = list_acp_backend_metadata().expect("list ACP metadata");
        let entry = metadata
            .iter()
            .find(|entry| entry.id == "registry_test")
            .expect("registry test metadata entry");
        assert_eq!(entry.api_version, 1);
        assert!(
            entry.capabilities.contains(&AcpCapability::TurnExecution),
            "metadata should include turn execution capability"
        );
    }

    #[test]
    fn describe_acp_backend_uses_default_when_id_absent() {
        let metadata = describe_acp_backend(None).expect("describe default ACP backend");
        assert_eq!(metadata.id, DEFAULT_ACP_BACKEND_ID);
    }

    #[test]
    fn resolve_acp_backend_returns_shared_backend_instance() {
        let first = resolve_acp_backend(Some(DEFAULT_ACP_BACKEND_ID)).expect("resolve first");
        let second = resolve_acp_backend(Some(DEFAULT_ACP_BACKEND_ID)).expect("resolve second");
        assert!(
            Arc::ptr_eq(&first, &second),
            "ACP registry should return shared backend instances to preserve runtime-local state"
        );
    }
}
