use loong_contracts::MemoryCoreRequest;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::*;

fn core_dispatch_test_lock() -> &'static Mutex<()> {
    static CORE_DISPATCH_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    CORE_DISPATCH_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(feature = "memory-sqlite")]
fn isolated_memory_workspace(prefix: &str) -> (PathBuf, runtime_config::MemoryRuntimeConfig) {
    let root = crate::test_support::unique_temp_dir(prefix);
    std::fs::create_dir_all(&root).expect("create isolated memory workspace");

    let db_path = root.join("memory.sqlite3");
    let config = sqlite_memory_config_with_window(db_path, 1);

    (root, config)
}

#[cfg(feature = "memory-sqlite")]
fn sqlite_memory_config(db_path: PathBuf) -> runtime_config::MemoryRuntimeConfig {
    runtime_config::MemoryRuntimeConfig::for_sqlite_path(db_path)
}

#[cfg(feature = "memory-sqlite")]
fn sqlite_memory_config_with_window(
    db_path: PathBuf,
    sliding_window: usize,
) -> runtime_config::MemoryRuntimeConfig {
    let mut config = sqlite_memory_config(db_path);
    config.sliding_window = sliding_window;
    config
}

#[cfg(feature = "memory-sqlite")]
fn sqlite_memory_config_with_profile(
    db_path: PathBuf,
    profile: crate::config::MemoryProfile,
    sliding_window: usize,
) -> runtime_config::MemoryRuntimeConfig {
    let mut config = sqlite_memory_config_with_window(db_path, sliding_window);
    config.profile = profile;
    config.mode = profile.mode();
    config
}

fn selected_memory_system_config(system_id: &str) -> runtime_config::MemoryRuntimeConfig {
    runtime_config::MemoryRuntimeConfig {
        resolved_system_id: Some(system_id.to_owned()),
        ..runtime_config::MemoryRuntimeConfig::default()
    }
}

#[cfg(feature = "memory-sqlite")]
fn cleanup_memory_workspace(workspace_root: &std::path::Path, db_path: &std::path::Path) {
    let _ = drop_cached_sqlite_runtime(db_path);
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_dir(workspace_root);
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

#[test]
fn supported_memory_core_operations_for_sqlite_are_stable() {
    let operations = supported_memory_core_operations(crate::config::MemoryBackendKind::Sqlite);
    let operation_names = operations
        .into_iter()
        .map(MemoryCoreOperation::as_str)
        .collect::<Vec<_>>();

    assert_eq!(
        operation_names,
        vec![
            "append_turn",
            "window",
            "transcript",
            "clear_session",
            "replace_turns",
            "read_context",
            "read_stage_envelope",
        ]
    );
}

#[tokio::test]
async fn mvp_memory_adapter_routes_through_kernel() {
    use std::collections::{BTreeMap, BTreeSet};

    use loong_contracts::Capability;
    use loong_kernel::{
        ExecutionRoute, HarnessKind, LoongKernel, StaticPolicyEngine, VerticalPackManifest,
    };

    let (mut kernel, _audit) = LoongKernel::new_with_in_memory_audit(StaticPolicyEngine::default());

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

    let tmp = std::env::temp_dir().join(format!("loong-test-memory-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("isolated-test.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config(db_path.clone());

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
        "loong-test-memory-window-semantics-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("window-semantics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config_with_window(db_path.clone(), 12);

    for idx in 0..130 {
        append_turn_direct(
            "window-semantics-session",
            "user",
            &format!("turn-{idx}"),
            &config,
        )
        .expect("append_turn_direct should succeed");
    }

    let explicit_limit_config = sqlite_memory_config_with_window(db_path.clone(), 1);
    let turns = window_direct("window-semantics-session", 2, &explicit_limit_config)
        .expect("window_direct should honor the explicit limit");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].content, "turn-128");
    assert_eq!(turns[1].content, "turn-129");

    let default_window_config = sqlite_memory_config_with_window(db_path.clone(), 3);
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

    let capped_window_config = sqlite_memory_config_with_window(db_path.clone(), 999);
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
fn memory_transcript_operation_returns_full_session_history() {
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "loong-test-memory-transcript-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("transcript.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config_with_window(db_path.clone(), 4);

    for idx in 0..7 {
        let role = if idx % 2 == 0 { "user" } else { "assistant" };
        append_turn_direct("transcript-session", role, &format!("turn-{idx}"), &config)
            .expect("append_turn_direct should succeed");
    }

    let transcript =
        execute_memory_core_with_config(build_transcript_request("transcript-session", 2), &config)
            .expect("transcript load should succeed");
    let transcript_turns: Vec<ConversationTurn> = serde_json::from_value(
        transcript
            .payload
            .get("turns")
            .cloned()
            .expect("turns payload should be present"),
    )
    .expect("turns payload should decode");

    assert_eq!(transcript.payload["page_size"], json!(2));
    assert_eq!(transcript.payload["turn_count"], json!(7));
    assert_eq!(transcript_turns.len(), 7);
    assert_eq!(
        transcript_turns.first().map(|turn| turn.content.as_str()),
        Some("turn-0")
    );
    assert_eq!(
        transcript_turns.last().map(|turn| turn.content.as_str()),
        Some("turn-6")
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn load_prompt_context_with_diagnostics_omits_legacy_identity_from_profile_projection() {
    use crate::config::MemoryProfile;
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "loong-test-memory-profile-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("profile-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Imported External Skills Artifacts\n- kind=skills_catalog\n- declared=custom/skill-a";
    let mut config =
        sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
    config.profile_note = Some(profile_note.to_owned());

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
fn load_prompt_context_with_diagnostics_projects_typed_personalization_without_profile_note() {
    use crate::config::MemoryProfile;

    let workspace_root = crate::test_support::unique_temp_dir(
        "loong-test-memory-profile-diagnostics-personalization",
    );
    std::fs::create_dir_all(&workspace_root).expect("create diagnostics workspace");

    let db_path = workspace_root.join("profile-diagnostics-personalization.sqlite3");
    let default_personalization = crate::config::PersonalizationConfig::default();
    let schema_version = default_personalization.schema_version;
    let personalization = crate::config::PersonalizationConfig {
        preferred_name: Some("Chum".to_owned()),
        response_density: Some(crate::config::ResponseDensity::Balanced),
        initiative_level: Some(crate::config::InitiativeLevel::AskBeforeActing),
        standing_boundaries: Some("Ask before destructive actions.".to_owned()),
        timezone: Some("Asia/Shanghai".to_owned()),
        locale: None,
        prompt_state: crate::config::PersonalizationPromptState::Configured,
        schema_version,
        updated_at_epoch_seconds: Some(1_775_095_200),
    };
    let mut config =
        sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
    config.profile_note = None;
    config.personalization = Some(personalization);

    append_turn_direct(
        "profile-diagnostics-personalization-session",
        "user",
        "recent turn",
        &config,
    )
    .expect("append_turn_direct should succeed");

    let diagnostics_context = load_prompt_context_with_diagnostics(
        "profile-diagnostics-personalization-session",
        &config,
    );
    let (entries, _diagnostics) =
        diagnostics_context.expect("load_prompt_context_with_diagnostics should succeed");
    let profile_entry = entries
        .iter()
        .find(|entry| entry.kind == MemoryContextKind::Profile)
        .expect("profile entry");
    let profile_content = profile_entry.content.as_str();

    assert!(profile_content.contains("## Session Profile"));
    assert!(profile_content.contains("Preferred name: Chum"));
    assert!(profile_content.contains("Response density: balanced"));
    assert!(profile_content.contains("Initiative level: ask_before_acting"));
    assert!(profile_content.contains("Ask before destructive actions."));
    assert!(profile_content.contains("Timezone: Asia/Shanghai"));
    assert!(!profile_content.contains("## Resolved Runtime Identity"));

    cleanup_memory_workspace(&workspace_root, &db_path);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn load_prompt_context_with_diagnostics_uses_selected_memory_system_id_in_provenance() {
    let (root, mut config) = isolated_memory_workspace("loong-selected-system-provenance");
    let session_id = "selected-system-provenance-session";

    config.resolved_system_id = Some(crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned());

    append_turn_direct(session_id, "user", "hello from selected system", &config)
        .expect("append_turn_direct should succeed");

    let (entries, _diagnostics) = load_prompt_context_with_diagnostics(session_id, &config)
        .expect("load_prompt_context_with_diagnostics should succeed");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provenance.len(), 1);
    assert_eq!(
        entries[0].provenance[0].memory_system_id,
        crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID
    );

    let sqlite_path = config.sqlite_path.expect("sqlite path");
    let _ = std::fs::remove_file(sqlite_path);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn pre_compaction_durable_flush_deduplicates_repeated_summary_exports() {
    let durable_flush_lock = crate::test_support::durable_memory_flush_test_lock();
    let _durable_flush_guard = durable_flush_lock.blocking_lock();
    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    let (workspace_root, config) = isolated_memory_workspace("loong-pre-compaction-durable-flush");

    append_turn_direct(
        "durable-flush-session",
        "user",
        "remember the deployment cutoff",
        &config,
    )
    .expect("append user turn");
    append_turn_direct(
        "durable-flush-session",
        "assistant",
        "deployment cutoff is tonight",
        &config,
    )
    .expect("append assistant turn");

    let first = super::durable_flush::flush_pre_compaction_durable_memory(
        "durable-flush-session",
        Some(workspace_root.as_path()),
        &config,
    );
    let first = runtime.block_on(first).expect("first durable flush");
    let first_path = match first {
        super::durable_flush::PreCompactionDurableFlushOutcome::Flushed { path, .. } => path,
        other @ super::durable_flush::PreCompactionDurableFlushOutcome::SkippedMissingWorkspaceRoot
        | other @ super::durable_flush::PreCompactionDurableFlushOutcome::SkippedNoSummary
        | other @ super::durable_flush::PreCompactionDurableFlushOutcome::SkippedDuplicate => {
            panic!("expected flushed outcome, got {other:?}")
        }
    };

    let second = super::durable_flush::flush_pre_compaction_durable_memory(
        "durable-flush-session",
        Some(workspace_root.as_path()),
        &config,
    );
    let second = runtime.block_on(second).expect("second durable flush");
    assert_eq!(
        second,
        super::durable_flush::PreCompactionDurableFlushOutcome::SkippedDuplicate
    );

    let exported = std::fs::read_to_string(&first_path).expect("read durable memory log");
    let marker_count = exported.matches("- content_sha256: ").count();
    assert_eq!(
        marker_count, 1,
        "duplicate flush should not append another entry"
    );
    assert!(exported.contains("Advisory durable recall"));
    assert!(exported.contains("remember the deployment cutoff"));
    assert!(!exported.contains("## Resolved Runtime Identity"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn pre_compaction_durable_flush_skips_when_no_summary_checkpoint_exists() {
    let durable_flush_lock = crate::test_support::durable_memory_flush_test_lock();
    let _durable_flush_guard = durable_flush_lock.blocking_lock();
    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    let (workspace_root, config) =
        isolated_memory_workspace("loong-pre-compaction-durable-flush-empty");

    append_turn_direct(
        "durable-flush-empty-session",
        "user",
        "only one turn",
        &config,
    )
    .expect("append user turn");

    let outcome = super::durable_flush::flush_pre_compaction_durable_memory(
        "durable-flush-empty-session",
        Some(workspace_root.as_path()),
        &config,
    );
    let outcome = runtime
        .block_on(outcome)
        .expect("durable flush without summary should succeed");

    assert_eq!(
        outcome,
        super::durable_flush::PreCompactionDurableFlushOutcome::SkippedNoSummary
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn append_turn_direct_bypasses_core_dispatch() {
    use std::fs;

    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let tmp = std::env::temp_dir().join(format!(
        "loong-test-memory-append-fast-path-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("append-fast-path.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config(db_path.clone());

    super::test_support::begin_core_dispatch_capture();
    append_turn_direct("append-fast-path-session", "user", "hello", &config)
        .expect("append_turn_direct should succeed");

    assert_eq!(
        super::test_support::core_dispatch_count(),
        0,
        "append_turn_direct should bypass core dispatch"
    );
    super::test_support::end_core_dispatch_capture();

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
        "loong-test-memory-window-fast-path-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("window-fast-path.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config(db_path.clone());

    append_turn_direct("window-fast-path-session", "user", "hello", &config)
        .expect("seed append_turn_direct should succeed");
    super::test_support::begin_core_dispatch_capture();

    let turns = window_direct("window-fast-path-session", 10, &config)
        .expect("window_direct should succeed");

    assert_eq!(turns.len(), 1);
    assert_eq!(
        super::test_support::core_dispatch_count(),
        0,
        "window_direct should bypass core dispatch"
    );
    super::test_support::end_core_dispatch_capture();

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn replace_session_turns_direct_rewrites_window() {
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "loong-test-memory-replace-turns-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("replace-turns.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config(db_path.clone());

    append_turn_direct("replace-turns-session", "user", "turn 1", &config)
        .expect("seed turn 1 should succeed");
    append_turn_direct("replace-turns-session", "assistant", "turn 2", &config)
        .expect("seed turn 2 should succeed");

    replace_session_turns_direct(
        "replace-turns-session",
        &[
            WindowTurn {
                role: "assistant".into(),
                content: "summary".into(),
                ts: Some(2),
            },
            WindowTurn {
                role: "user".into(),
                content: "recent".into(),
                ts: Some(3),
            },
        ],
        &config,
    )
    .expect("replace_session_turns_direct should succeed");

    let turns = window_direct("replace-turns-session", 10, &config)
        .expect("window_direct should read rewritten turns");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].content, "summary");
    assert_eq!(turns[1].content, "recent");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn replace_session_turns_direct_requires_explicit_timestamps() {
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "loong-test-memory-replace-turns-ts-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("replace-turns-missing-ts.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_memory_config(db_path.clone());

    let error = replace_session_turns_direct(
        "replace-turns-session",
        &[WindowTurn {
            role: "assistant".into(),
            content: "summary".into(),
            ts: None,
        }],
        &config,
    )
    .expect_err("replace_session_turns_direct should require explicit timestamps");

    assert!(error.contains("turns[*].ts"), "unexpected error: {error}");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn registry_selected_system_can_override_memory_runtime_execution() {
    use async_trait::async_trait;

    struct RuntimeExecutingMemorySystem;

    struct RuntimeExecutingMemorySystemRuntime {
        metadata: MemorySystemMetadata,
    }

    impl MemorySystem for RuntimeExecutingMemorySystem {
        fn id(&self) -> &'static str {
            "registry-runtime-executing"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-runtime-executing",
                [MemorySystemCapability::PromptHydration],
                "Registry runtime-executing test system",
            )
            .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve])
        }

        fn create_runtime(
            &self,
            _config: &runtime_config::MemoryRuntimeConfig,
        ) -> crate::CliResult<Option<Box<dyn MemorySystemRuntime>>> {
            let metadata = self.metadata();
            let runtime = RuntimeExecutingMemorySystemRuntime { metadata };
            let boxed_runtime: Box<dyn MemorySystemRuntime> = Box::new(runtime);

            Ok(Some(boxed_runtime))
        }
    }

    #[async_trait]
    impl MemorySystemRuntime for RuntimeExecutingMemorySystemRuntime {
        fn metadata(&self) -> &MemorySystemMetadata {
            &self.metadata
        }

        fn supported_core_operations(&self) -> Vec<MemoryCoreOperation> {
            vec![MemoryCoreOperation::ReadContext]
        }

        fn execute_core(&self, request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
            let operation = request.operation;
            let payload = json!({
                "adapter": "registry-runtime-executing",
                "operation": operation,
            });
            let outcome = MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload,
            };

            Ok(outcome)
        }

        fn hydrate_stage_envelope(
            &self,
            _session_id: &str,
            _workspace_root: Option<&std::path::Path>,
        ) -> Result<StageEnvelope, String> {
            let diagnostics = MemoryDiagnostics {
                system_id: self.metadata.id.to_owned(),
                fail_open: true,
                strict_mode_requested: false,
                strict_mode_active: false,
                degraded: false,
                derivation_error: None,
                retrieval_error: None,
                rank_error: None,
                recent_window_count: 0,
                entry_count: 0,
            };
            let hydrated = HydratedMemoryContext {
                entries: Vec::new(),
                recent_window: Vec::new(),
                diagnostics,
            };
            let envelope = StageEnvelope {
                hydrated,
                retrieval_request: None,
                retrieval_planner_snapshot: None,
                diagnostics: vec![StageDiagnostics::succeeded(MemoryStageFamily::Retrieve)],
            };

            Ok(envelope)
        }

        async fn run_compact_stage(
            &self,
            _session_id: &str,
            _workspace_root: Option<&std::path::Path>,
        ) -> Result<StageDiagnostics, String> {
            let diagnostics = StageDiagnostics::succeeded(MemoryStageFamily::Compact);
            Ok(diagnostics)
        }
    }

    fn ensure_registry_runtime_executing_system_registered() {
        static REGISTRY_RUNTIME_EXECUTING_SYSTEM: OnceLock<()> = OnceLock::new();
        REGISTRY_RUNTIME_EXECUTING_SYSTEM.get_or_init(|| {
            register_memory_system("registry-runtime-executing", || {
                Box::new(RuntimeExecutingMemorySystem)
            })
            .expect("register runtime-executing memory system");
        });
    }

    let _guard = core_dispatch_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    ensure_registry_runtime_executing_system_registered();

    let config = selected_memory_system_config("registry-runtime-executing");

    let request = MemoryCoreRequest {
        operation: "noop".to_owned(),
        payload: json!({}),
    };
    let outcome = execute_memory_core_with_config(request, &config)
        .expect("registry runtime should handle memory core execution");

    assert_eq!(outcome.payload["adapter"], "registry-runtime-executing");

    let envelope = hydrate_stage_envelope("runtime-executing-session", &config)
        .expect("registry runtime should handle stage envelope hydration");

    assert_eq!(
        envelope.hydrated.diagnostics.system_id,
        "registry-runtime-executing"
    );
}

#[tokio::test]
async fn registry_selected_system_can_use_compact_stage_hook_without_custom_runtime() {
    struct CompactHookMemorySystem;

    static EXPECTED_COMPACT_HOOK_SESSION_ID: OnceLock<String> = OnceLock::new();
    static EXPECTED_COMPACT_HOOK_WORKSPACE_ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();

    impl MemorySystem for CompactHookMemorySystem {
        fn id(&self) -> &'static str {
            "registry-compact-hook"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-compact-hook",
                [MemorySystemCapability::PromptHydration],
                "Registry compact hook test system",
            )
            .with_runtime_fallback_kind(
                crate::memory::MemorySystemRuntimeFallbackKind::SystemBacked,
            )
            .with_supported_stage_families([MemoryStageFamily::Compact])
        }

        fn run_compact_stage(
            &self,
            session_id: &str,
            workspace_root: Option<&std::path::Path>,
            _config: &runtime_config::MemoryRuntimeConfig,
        ) -> Result<Option<StageDiagnostics>, String> {
            let expected_session_id = EXPECTED_COMPACT_HOOK_SESSION_ID
                .get()
                .expect("expected compact-hook session id");
            let expected_workspace_root = EXPECTED_COMPACT_HOOK_WORKSPACE_ROOT
                .get()
                .expect("expected compact-hook workspace root");
            let actual_workspace_root = workspace_root.map(|path| path.to_path_buf());
            let expected_workspace_root = Some(expected_workspace_root.clone());
            if session_id != expected_session_id {
                let error = format!(
                    "expected compact hook session id `{expected_session_id}`, got `{session_id}`"
                );

                return Err(error);
            }
            if actual_workspace_root != expected_workspace_root {
                let error = format!(
                    "expected compact hook workspace root `{:?}`, got `{:?}`",
                    expected_workspace_root, actual_workspace_root
                );

                return Err(error);
            }
            let diagnostics = StageDiagnostics::succeeded(MemoryStageFamily::Compact);

            Ok(Some(diagnostics))
        }
    }

    fn ensure_registry_compact_hook_system_registered() {
        static REGISTRY_COMPACT_HOOK_SYSTEM: OnceLock<()> = OnceLock::new();
        REGISTRY_COMPACT_HOOK_SYSTEM.get_or_init(|| {
            register_memory_system("registry-compact-hook", || {
                Box::new(CompactHookMemorySystem)
            })
            .expect("register compact-hook memory system");
        });
    }

    ensure_registry_compact_hook_system_registered();

    let workspace_root = crate::test_support::unique_temp_dir("compact-hook-workspace");
    std::fs::create_dir_all(&workspace_root).expect("create compact hook workspace");
    EXPECTED_COMPACT_HOOK_SESSION_ID
        .set("compact-hook-session".to_owned())
        .ok();
    EXPECTED_COMPACT_HOOK_WORKSPACE_ROOT
        .set(workspace_root.clone())
        .ok();

    let config = selected_memory_system_config("registry-compact-hook");
    let diagnostics = run_compact_stage(
        "compact-hook-session",
        Some(workspace_root.as_path()),
        &config,
    )
    .await
    .expect("compact stage should run through system-backed runtime");

    assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
    assert_eq!(diagnostics.outcome, StageOutcome::Succeeded);
    assert!(!diagnostics.fallback_activated);
}
