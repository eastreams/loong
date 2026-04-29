use super::*;

#[test]
fn memory_system_metadata_json_includes_stage_families_summary_and_source() {
    use mvp::memory::MemorySystem as _;

    let metadata = mvp::memory::BuiltinMemorySystem.metadata();
    let payload = memory_system_metadata_json(&metadata, Some("default"));

    assert_eq!(payload["id"], "builtin");
    assert_eq!(payload["api_version"], 1);
    assert_eq!(payload["source"], "default");
    assert!(
        payload["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("Built-in")
    );
    assert!(
        payload["capabilities"]
            .as_array()
            .expect("capabilities should be an array")
            .iter()
            .any(|entry| entry == "canonical_store")
    );
    assert_eq!(payload["runtime_fallback_kind"], "metadata_only");
    assert_eq!(
        payload["supported_stage_families"],
        json!(["derive", "retrieve", "rank", "compact"])
    );
    assert_eq!(
        payload["supported_pre_assembly_stage_families"],
        json!(["derive", "retrieve", "rank"])
    );
    assert_eq!(
        payload["supported_recall_modes"],
        json!(["prompt_assembly", "operator_inspection"])
    );
}

#[test]
fn build_memory_systems_cli_json_payload_includes_runtime_policy() {
    let config = mvp::config::LoongConfig {
        memory: mvp::config::MemoryConfig {
            profile: mvp::config::MemoryProfile::WindowPlusSummary,
            fail_open: false,
            ingest_mode: mvp::config::MemoryIngestMode::AsyncBackground,
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongConfig::default()
    };
    let snapshot =
        mvp::memory::collect_memory_system_runtime_snapshot(&config).expect("runtime snapshot");

    let payload = build_memory_systems_cli_json_payload("/tmp/loong.toml", &snapshot);

    assert_eq!(payload["config"], "/tmp/loong.toml");
    assert_eq!(payload["selected"]["id"], "builtin");
    assert_eq!(payload["selected"]["source"], "default");
    assert_eq!(
        payload["selected"]["runtime_fallback_kind"],
        "metadata_only"
    );
    assert_eq!(
        payload["selected"]["supported_stage_families"],
        json!(["derive", "retrieve", "rank", "compact"])
    );
    assert_eq!(
        payload["selected"]["supported_pre_assembly_stage_families"],
        json!(["derive", "retrieve", "rank"])
    );
    assert_eq!(
        payload["selected"]["supported_recall_modes"],
        json!(["prompt_assembly", "operator_inspection"])
    );
    assert_eq!(
        payload["core_operations"],
        json!([
            "append_turn",
            "window",
            "transcript",
            "clear_session",
            "replace_turns",
            "read_context",
            "read_stage_envelope"
        ])
    );
    assert_eq!(payload["policy"]["backend"], "sqlite");
    assert_eq!(payload["policy"]["profile"], "window_plus_summary");
    assert_eq!(payload["policy"]["mode"], "window_plus_summary");
    assert_eq!(payload["policy"]["ingest_mode"], "async_background");
    assert_eq!(payload["policy"]["fail_open"], false);
    assert_eq!(payload["policy"]["strict_mode_requested"], true);
    assert_eq!(payload["policy"]["strict_mode_active"], false);
    assert_eq!(payload["policy"]["effective_fail_open"], true);
}

#[test]
fn render_memory_system_snapshot_text_reports_fail_open_policy() {
    let mut env = loong_daemon::test_support::ScopedEnv::new();
    for key in [
        "LOONG_MEMORY_BACKEND",
        "LOONG_MEMORY_SYSTEM",
        "LOONG_MEMORY_PROFILE",
        "LOONG_MEMORY_FAIL_OPEN",
        "LOONG_MEMORY_INGEST_MODE",
        "LOONG_SQLITE_PATH",
        "LOONG_SLIDING_WINDOW",
        "LOONG_MEMORY_SUMMARY_MAX_CHARS",
        "LOONG_MEMORY_PROFILE_NOTE",
    ] {
        env.remove(key);
    }
    let config = mvp::config::LoongConfig {
        memory: mvp::config::MemoryConfig {
            profile: mvp::config::MemoryProfile::WindowPlusSummary,
            fail_open: false,
            ingest_mode: mvp::config::MemoryIngestMode::AsyncBackground,
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongConfig::default()
    };
    let snapshot =
        mvp::memory::collect_memory_system_runtime_snapshot(&config).expect("runtime snapshot");

    let rendered = render_memory_system_snapshot_text("/tmp/loong.toml", &snapshot);

    assert!(rendered.contains("config=/tmp/loong.toml"));
    assert!(rendered.contains(
        "selected=builtin source=default api_version=1 capabilities=canonical_store,deterministic_summary,profile_note_projection,prompt_hydration,retrieval_provenance runtime_fallback_kind=metadata_only stages=derive,retrieve,rank,compact pre_assembly_stages=derive,retrieve,rank recall_modes=prompt_assembly,operator_inspection core_operations=append_turn,window,transcript,clear_session,replace_turns,read_context,read_stage_envelope"
    ));
    assert!(rendered.contains("policy=backend:sqlite profile:window_plus_summary mode:window_plus_summary ingest_mode:async_background fail_open:false strict_mode_requested:true strict_mode_active:false effective_fail_open:true"));
    assert!(rendered.contains(
        "- builtin api_version=1 capabilities=canonical_store,deterministic_summary,profile_note_projection,prompt_hydration,retrieval_provenance runtime_fallback_kind=metadata_only stages=derive,retrieve,rank,compact pre_assembly_stages=derive,retrieve,rank recall_modes=prompt_assembly,operator_inspection"
    ));
    assert!(rendered.contains(
        "- recall_first api_version=1 capabilities=prompt_hydration,retrieval_provenance runtime_fallback_kind=system_backed stages=derive,retrieve,rank pre_assembly_stages=derive,retrieve,rank recall_modes=prompt_assembly"
    ));
}
