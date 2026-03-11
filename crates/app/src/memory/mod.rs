#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::json;

mod kernel_adapter;
pub mod runtime_config;
#[cfg(feature = "memory-sqlite")]
mod sqlite;

pub use kernel_adapter::MvpMemoryAdapter;
#[cfg(feature = "memory-sqlite")]
pub use sqlite::ConversationTurn;

pub fn execute_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    execute_memory_core_with_config(request, runtime_config::get_memory_runtime_config())
}

pub fn execute_memory_core_with_config(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    match request.operation.as_str() {
        "append_turn" => append_turn(request, config),
        "window" => load_window(request, config),
        "clear_session" => clear_session(request, config),
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

fn append_turn(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::append_turn(request, config)
    }
}

fn load_window(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::load_window(request, config)
    }
}

fn clear_session(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::clear_session(request, config)
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn append_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<(), String> {
    sqlite::append_turn_direct(session_id, role, content, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_direct(
    session_id: &str,
    limit: usize,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    sqlite::window_direct(session_id, limit, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    sqlite::ensure_memory_db_ready(path, config)
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

        kernel.register_core_memory_adapter(MvpMemoryAdapter::new());
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

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn memory_write_read_round_trip_uses_injected_config() {
        use std::fs;

        let tmp =
            std::env::temp_dir().join(format!("loongclaw-test-memory-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("isolated-test.sqlite3");
        // Ensure clean state
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        append_turn_direct("rt-session", "user", "hello from test", &config)
            .expect("append_turn_direct should succeed");

        let turns = window_direct("rt-session", 10, &config).expect("window_direct should succeed");

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello from test");

        // The isolated DB was created at the injected path
        assert!(
            db_path.exists(),
            "sqlite file should exist at injected path"
        );

        // Cleanup
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn explicit_window_limit_ignores_env_default() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-window-limit-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("limit-override.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        for idx in 0..3 {
            append_turn_direct(
                "window-limit-session",
                "user",
                &format!("turn-{idx}"),
                &config,
            )
            .expect("append_turn_direct should succeed");
        }

        std::env::set_var("LOONGCLAW_SLIDING_WINDOW", "1");
        let turns = window_direct("window-limit-session", 2, &config)
            .expect("window_direct should honor the explicit limit");
        std::env::remove_var("LOONGCLAW_SLIDING_WINDOW");

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "turn-1");
        assert_eq!(turns[1].content, "turn-2");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn missing_window_limit_uses_env_default_and_caps_high_limits() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-window-default-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("default-and-cap.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        for idx in 0..130 {
            append_turn_direct(
                "window-default-session",
                "user",
                &format!("turn-{idx}"),
                &config,
            )
            .expect("append_turn_direct should succeed");
        }

        std::env::set_var("LOONGCLAW_SLIDING_WINDOW", "3");
        let default_window = execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: "window".to_owned(),
                payload: json!({
                    "session_id": "window-default-session",
                }),
            },
            &config,
        )
        .expect("window load without explicit limit should succeed");
        std::env::remove_var("LOONGCLAW_SLIDING_WINDOW");

        let default_turns: Vec<ConversationTurn> = serde_json::from_value(
            default_window
                .payload
                .get("turns")
                .cloned()
                .expect("turns payload should be present"),
        )
        .expect("turns payload should decode");
        assert_eq!(default_turns.len(), 3);
        assert_eq!(default_window.payload["limit"], json!(3));

        let capped_turns = window_direct("window-default-session", 999, &config)
            .expect("window_direct should clamp large limits");
        assert_eq!(capped_turns.len(), 128);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }
}
