#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::{Value, json};

use crate::config::{MemoryBackendKind, MemoryMode};

mod kernel_adapter;
pub mod runtime_config;
#[cfg(feature = "memory-sqlite")]
mod sqlite;

pub use kernel_adapter::MvpMemoryAdapter;
#[cfg(feature = "memory-sqlite")]
pub use sqlite::ConversationTurn;

pub const MEMORY_OP_APPEND_TURN: &str = "append_turn";
pub const MEMORY_OP_WINDOW: &str = "window";
pub const MEMORY_OP_CLEAR_SESSION: &str = "clear_session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTurn {
    pub role: String,
    pub content: String,
    pub ts: Option<i64>,
}

pub fn build_append_turn_request(session_id: &str, role: &str, content: &str) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_APPEND_TURN.to_owned(),
        payload: json!({
            "session_id": session_id,
            "role": role,
            "content": content,
        }),
    }
}

pub fn build_window_request(session_id: &str, limit: usize) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_WINDOW.to_owned(),
        payload: json!({
            "session_id": session_id,
            "limit": limit,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryContextKind {
    Profile,
    Summary,
    Turn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContextEntry {
    pub kind: MemoryContextKind,
    pub role: String,
    pub content: String,
}

pub fn execute_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    execute_memory_core_with_config(request, runtime_config::get_memory_runtime_config())
}

pub fn execute_memory_core_with_config(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    match config.backend {
        MemoryBackendKind::Sqlite => match request.operation.as_str() {
            MEMORY_OP_APPEND_TURN => append_turn(request, config),
            MEMORY_OP_WINDOW => load_window(request, config),
            MEMORY_OP_CLEAR_SESSION => clear_session(request, config),
            _ => Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "kv-core",
                    "operation": request.operation,
                    "payload": request.payload,
                }),
            }),
        },
    }
}

pub fn decode_window_turns(payload: &Value) -> Vec<WindowTurn> {
    payload
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|turn| WindowTurn {
            role: turn
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
                .to_owned(),
            content: turn
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            ts: turn.get("ts").and_then(Value::as_i64),
        })
        .collect()
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
pub fn window_direct_extended(
    session_id: &str,
    limit: usize,
) -> Result<Vec<ConversationTurn>, String> {
    sqlite::window_direct_with_options(
        session_id,
        limit,
        true,
        runtime_config::get_memory_runtime_config(),
    )
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    sqlite::ensure_memory_db_ready(path, config)
}

pub fn load_prompt_context(
    session_id: &str,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<Vec<MemoryContextEntry>, String> {
    let mut entries = Vec::new();

    if matches!(config.mode, MemoryMode::ProfilePlusWindow)
        && let Some(profile_note) = config
            .profile_note
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        entries.push(MemoryContextEntry {
            kind: MemoryContextKind::Profile,
            role: "system".to_owned(),
            content: format!(
                "## Session Profile\nDurable preferences or imported identity carried into this session:\n- {profile_note}"
            ),
        });
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let turns = sqlite::window_direct(session_id, config.sliding_window, config)?;
        if matches!(config.mode, MemoryMode::WindowPlusSummary) {
            let all_turns = sqlite::session_turns_direct(session_id, config)?;
            let older_turn_count = all_turns.len().saturating_sub(turns.len());
            if older_turn_count > 0
                && let Some(older_turns) = all_turns.get(..older_turn_count)
                && let Some(summary) = build_summary_block(older_turns, config.summary_max_chars)
            {
                entries.push(MemoryContextEntry {
                    kind: MemoryContextKind::Summary,
                    role: "system".to_owned(),
                    content: summary,
                });
            }
        }
        for turn in turns {
            entries.push(MemoryContextEntry {
                kind: MemoryContextKind::Turn,
                role: turn.role,
                content: turn.content,
            });
        }
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = session_id;
    }

    Ok(entries)
}

#[cfg(feature = "memory-sqlite")]
fn build_summary_block(turns: &[ConversationTurn], max_chars: usize) -> Option<String> {
    if turns.is_empty() {
        return None;
    }

    let header = "## Memory Summary\nEarlier session context condensed from turns outside the active window:";
    let mut body = String::new();
    let budget = max_chars.max(256);

    for turn in turns {
        let normalized = turn
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if normalized.is_empty() {
            continue;
        }
        let prefix = if body.is_empty() { "" } else { "\n" };
        let line = format!("{prefix}- {}: {}", turn.role, normalized);
        if body.len() + line.len() > budget {
            let remaining = budget.saturating_sub(body.len());
            if remaining == 0 {
                break;
            }
            let truncated = line.chars().take(remaining).collect::<String>();
            body.push_str(&truncated);
            break;
        }
        body.push_str(&line);
    }

    if body.is_empty() {
        return None;
    }

    Some(format!("{header}\n{body}"))
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

    #[test]
    fn decode_window_turns_tolerates_partial_payload_shape() {
        let payload = json!({
            "turns": [
                {"role": "user", "content": "hello", "ts": 1},
                {"role": "assistant"},
                {"content": "only-content"},
                {}
            ]
        });
        let turns = decode_window_turns(&payload);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello");
        assert_eq!(turns[0].ts, Some(1));
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].content, "");
        assert_eq!(turns[2].role, "assistant");
        assert_eq!(turns[2].content, "only-content");
        assert_eq!(turns[3].role, "assistant");
        assert_eq!(turns[3].content, "");
    }

    #[test]
    fn decode_window_turns_returns_empty_for_missing_turns() {
        assert!(decode_window_turns(&json!({})).is_empty());
        assert!(decode_window_turns(&json!({"turns": null})).is_empty());
        assert!(decode_window_turns(&json!({"turns": "invalid"})).is_empty());
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
            ..runtime_config::MemoryRuntimeConfig::default()
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
    fn memory_window_limit_semantics_cover_explicit_fallback_and_bounds() {
        use std::fs;

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-test-memory-window-semantics-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-semantics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            sliding_window: 12,
            ..runtime_config::MemoryRuntimeConfig::default()
        };

        for idx in 0..130 {
            append_turn_direct(
                "window-semantics-session",
                "user",
                &format!("turn-{idx}"),
                &config,
            )
            .expect("append_turn_direct should succeed");
        }

        let explicit_limit_config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            sliding_window: 1,
            ..runtime_config::MemoryRuntimeConfig::default()
        };
        let turns = window_direct("window-semantics-session", 2, &explicit_limit_config)
            .expect("window_direct should honor the explicit limit");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "turn-128");
        assert_eq!(turns[1].content, "turn-129");

        let default_window_config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            sliding_window: 3,
            ..runtime_config::MemoryRuntimeConfig::default()
        };
        let default_window = execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: "window".to_owned(),
                payload: json!({
                    "session_id": "window-semantics-session",
                }),
            },
            &default_window_config,
        )
        .expect("window load without explicit limit should succeed");
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
        assert_eq!(default_turns[0].content, "turn-127");
        assert_eq!(default_turns[2].content, "turn-129");

        let capped_window_config = runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            sliding_window: 999,
            ..runtime_config::MemoryRuntimeConfig::default()
        };
        let capped_window = execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: "window".to_owned(),
                payload: json!({
                    "session_id": "window-semantics-session",
                }),
            },
            &capped_window_config,
        )
        .expect("window load without explicit limit should clamp high defaults");
        let capped_turns: Vec<ConversationTurn> = serde_json::from_value(
            capped_window
                .payload
                .get("turns")
                .cloned()
                .expect("turns payload should be present"),
        )
        .expect("turns payload should decode");
        assert_eq!(capped_turns.len(), 128);
        assert_eq!(capped_window.payload["limit"], json!(128));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn window_plus_summary_includes_condensed_older_context() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp =
            std::env::temp_dir().join(format!("loongclaw-summary-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("summary-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("summary-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("summary-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        append_turn_direct("summary-session", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        let hydrated =
            load_prompt_context("summary-session", &config).expect("load prompt context");

        assert!(
            hydrated
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Summary),
            "expected a summary entry"
        );
        assert!(
            hydrated
                .iter()
                .any(|entry| entry.content.contains("turn 1")),
            "expected summary to mention older turns"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_includes_profile_note_block() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp =
            std::env::temp_dir().join(format!("loongclaw-profile-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            profile_note: Some("Imported ZeroClaw preferences".to_owned()),
            ..runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("profile-session", "user", "recent turn", &config)
            .expect("append turn should succeed");

        let hydrated =
            load_prompt_context("profile-session", &config).expect("load prompt context");

        assert!(
            hydrated
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Profile),
            "expected a profile entry"
        );
        assert!(
            hydrated
                .iter()
                .any(|entry| entry.content.contains("Imported ZeroClaw preferences")),
            "expected profile note content"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }
}
