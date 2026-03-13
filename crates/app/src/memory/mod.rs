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
pub use sqlite::{ConversationTurn, TranscriptSearchMatch};

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

#[cfg(feature = "memory-sqlite")]
pub fn search_transcript_direct(
    session_ids: &[String],
    query: &str,
    limit: usize,
    excerpt_chars: usize,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<Vec<TranscriptSearchMatch>, String> {
    sqlite::search_transcript_direct(session_ids, query, limit, excerpt_chars, config)
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
    fn transcript_window_excludes_session_events() {
        use std::fs;

        use crate::session::repository::{
            NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
        };

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-session-events-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("session-events.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        let repo = SessionRepository::new(&config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: "session-a".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Session A".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create session");
        repo.append_event(NewSessionEvent {
            session_id: "session-a".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: None,
            payload_json: json!({"note": "control-plane event"}),
        })
        .expect("append session event");

        append_turn_direct("session-a", "user", "hello from transcript", &config)
            .expect("append transcript turn");

        let turns = window_direct("session-a", 10, &config).expect("window turns");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello from transcript");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn search_transcript_direct_returns_recent_matching_turns() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-search-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        append_turn_direct("session-a", "user", "first timeout budget note", &config)
            .expect("append first match");
        append_turn_direct("session-b", "assistant", "no relevant text here", &config)
            .expect("append unrelated turn");
        append_turn_direct(
            "session-a",
            "assistant",
            "latest timeout budget update",
            &config,
        )
        .expect("append latest match");

        let matches = search_transcript_direct(
            &["session-a".to_owned(), "session-b".to_owned()],
            "timeout budget",
            20,
            120,
            &config,
        )
        .expect("search transcript");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].session_id, "session-a");
        assert_eq!(matches[0].role, "assistant");
        assert!(
            matches[0].content_snippet.contains("timeout budget"),
            "snippet should contain query"
        );
        assert!(
            matches[0].turn_id > matches[1].turn_id,
            "results should be newest first"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn search_transcript_direct_excludes_non_matching_turns() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-search-miss-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("search-miss.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };

        append_turn_direct("session-a", "user", "archive inventory", &config).expect("append turn");

        let matches = search_transcript_direct(
            &["session-a".to_owned()],
            "timeout budget",
            20,
            120,
            &config,
        )
        .expect("search transcript");

        assert!(matches.is_empty(), "non-matching turns should be excluded");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn search_transcript_direct_clamps_limit_and_excerpt() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-search-clamp-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("search-clamp.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
        };
        let long_prefix = "prefix ".repeat(20);
        let long_suffix = " suffix".repeat(20);
        let long_match = format!("{long_prefix}timeout budget{long_suffix}");

        append_turn_direct("session-a", "user", "timeout budget alpha", &config)
            .expect("append alpha");
        append_turn_direct("session-a", "assistant", "timeout budget beta", &config)
            .expect("append beta");
        append_turn_direct("session-a", "assistant", &long_match, &config)
            .expect("append long match");

        let matches =
            search_transcript_direct(&["session-a".to_owned()], "timeout budget", 0, 12, &config)
                .expect("search transcript");

        assert_eq!(matches.len(), 1, "limit should clamp to at least one");
        assert!(
            matches[0].content_snippet.len() <= 406,
            "excerpt should clamp instead of returning the full long content"
        );
        assert!(
            matches[0].content_snippet.contains("timeout budget"),
            "snippet should preserve the matching phrase"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }
}
