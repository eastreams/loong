#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::json;

mod kernel_adapter;
#[cfg(feature = "memory-sqlite")]
mod sqlite;

pub use kernel_adapter::MvpMemoryAdapter;
#[cfg(feature = "memory-sqlite")]
pub use sqlite::ConversationTurn;

pub fn execute_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    match request.operation.as_str() {
        "append_turn" => append_turn(request),
        "window" => load_window(request),
        "clear_session" => clear_session(request),
        _ => Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "kv-core",
                "operation": request.operation,
                "payload": request.payload,
            }),
        }),
    }
}

fn append_turn(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = request;
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::append_turn(request)
    }
}

fn load_window(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = request;
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::load_window(request)
    }
}

fn clear_session(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = request;
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::clear_session(request)
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn append_turn_direct(session_id: &str, role: &str, content: &str) -> Result<(), String> {
    sqlite::append_turn_direct(session_id, role, content)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_direct(session_id: &str, limit: usize) -> Result<Vec<ConversationTurn>, String> {
    sqlite::window_direct(session_id, limit)
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_memory_db_ready(path: Option<PathBuf>) -> Result<PathBuf, String> {
    sqlite::ensure_memory_db_ready(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_memory_operation_stays_compatible() {
        let outcome = execute_memory_core(MemoryCoreRequest {
            operation: "noop".to_owned(),
            payload: json!({"a":1}),
        })
        .expect("fallback operation should succeed");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["adapter"], "kv-core");
    }

    #[tokio::test]
    async fn mvp_memory_adapter_routes_through_kernel() {
        use std::collections::{BTreeMap, BTreeSet};

        use loongclaw_contracts::Capability;
        use loongclaw_kernel::{
            ExecutionRoute, HarnessKind, LoongClawKernel, StaticPolicyEngine, VerticalPackManifest,
        };

        let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());

        kernel.register_core_memory_adapter(MvpMemoryAdapter);
        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .expect("set default memory adapter");

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "test".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::MemoryRead, Capability::MemoryWrite]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        // Use a fallback operation so it works regardless of memory-sqlite feature
        let request = MemoryCoreRequest {
            operation: "noop".to_owned(),
            payload: json!({"test": true}),
        };

        let caps = BTreeSet::from([Capability::MemoryRead]);
        let outcome = kernel
            .execute_memory_core("test-pack", &token, &caps, None, request)
            .await
            .expect("kernel memory core execution should succeed");

        assert_eq!(outcome.status, "ok");
    }
}
