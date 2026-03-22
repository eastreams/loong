use loongclaw_contracts::MemoryCoreRequest;
use serde_json::json;
use std::sync::{Mutex, OnceLock};

use super::*;

fn core_dispatch_test_lock() -> &'static Mutex<()> {
    static CORE_DISPATCH_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    CORE_DISPATCH_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

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

    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());

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

    let tmp = std::env::temp_dir().join(format!("loongclaw-test-memory-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("isolated-test.sqlite3");
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
    assert!(
        db_path.exists(),
        "sqlite file should exist at injected path"
    );

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
            operation: MEMORY_OP_WINDOW.to_owned(),
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
            operation: MEMORY_OP_WINDOW.to_owned(),
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
fn load_prompt_context_with_diagnostics_omits_legacy_identity_from_profile_projection() {
    use crate::config::{MemoryMode, MemoryProfile};
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-test-memory-profile-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("profile-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Imported External Skills Artifacts\n- kind=skills_catalog\n- declared=custom/skill-a";
    let config = runtime_config::MemoryRuntimeConfig {
        profile: MemoryProfile::ProfilePlusWindow,
        mode: MemoryMode::ProfilePlusWindow,
        sqlite_path: Some(db_path.clone()),
        sliding_window: 2,
        profile_note: Some(profile_note.to_owned()),
        ..runtime_config::MemoryRuntimeConfig::default()
    };

    append_turn_direct(
        "profile-diagnostics-session",
        "user",
        "recent turn",
        &config,
    )
    .expect("append_turn_direct should succeed");

    let (entries, _diagnostics) =
        load_prompt_context_with_diagnostics("profile-diagnostics-session", &config)
            .expect("load_prompt_context_with_diagnostics should succeed");
    let profile_entry = entries
        .iter()
        .find(|entry| entry.kind == MemoryContextKind::Profile)
        .expect("profile entry");

    assert!(
        profile_entry
            .content
            .contains("Imported External Skills Artifacts")
    );
    assert!(!profile_entry.content.contains("Legacy build copilot"));

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn append_turn_direct_bypasses_core_dispatch() {
    use std::fs;

    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-test-memory-append-fast-path-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("append-fast-path.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
        ..runtime_config::MemoryRuntimeConfig::default()
    };

    begin_core_dispatch_capture_for_tests();
    append_turn_direct("append-fast-path-session", "user", "hello", &config)
        .expect("append_turn_direct should succeed");

    assert_eq!(
        core_dispatch_count_for_tests(),
        0,
        "append_turn_direct should bypass core dispatch"
    );
    end_core_dispatch_capture_for_tests();

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn window_direct_bypasses_core_dispatch() {
    use std::fs;

    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-test-memory-window-fast-path-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("window-fast-path.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
        ..runtime_config::MemoryRuntimeConfig::default()
    };

    append_turn_direct("window-fast-path-session", "user", "hello", &config)
        .expect("seed append_turn_direct should succeed");
    begin_core_dispatch_capture_for_tests();

    let turns = window_direct("window-fast-path-session", 10, &config)
        .expect("window_direct should succeed");

    assert_eq!(turns.len(), 1);
    assert_eq!(
        core_dispatch_count_for_tests(),
        0,
        "window_direct should bypass core dispatch"
    );
    end_core_dispatch_capture_for_tests();

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}
