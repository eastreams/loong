use super::*;

#[test]
fn memory_context_benchmark_rejects_history_not_exceeding_window() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-invalid.json");

    let error = run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        None,
        8,
        8,
        256,
        8,
        2,
        4,
        1,
        1,
        false,
        1.10,
    )
    .expect_err("history equal to window should be rejected");

    assert!(error.contains("history_turns must exceed sliding_window"));

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn memory_context_benchmark_rejects_history_without_shrink_catch_up_headroom() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-shrink-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-shrink-invalid.json");

    let error = run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        None,
        9,
        8,
        256,
        8,
        2,
        4,
        1,
        1,
        false,
        1.10,
    )
    .expect_err("history with only one turn beyond the window should be rejected");

    assert!(error.contains("shrink catch-up mode"));

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn memory_context_benchmark_writes_report_with_all_scenarios() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-report-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-report.json");
    let temp_root = tmp.join("benchmark-temp-root");

    run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        Some(temp_root.to_str().expect("utf-8 temp root path")),
        24,
        6,
        256,
        12,
        2,
        4,
        1,
        2,
        false,
        1.10,
    )
    .expect("memory context benchmark should write report");

    let report_raw = fs::read_to_string(&output).expect("read benchmark report");
    let report: Value = serde_json::from_str(&report_raw).expect("benchmark report JSON");

    assert_eq!(report.get("history_turns"), Some(&json!(24)));
    assert_eq!(report.get("suite_repetitions"), Some(&json!(2)));
    assert_eq!(
        report.get("suite_aggregation"),
        Some(&json!("median_of_suite_p95"))
    );
    assert_eq!(
        report.get("rss_telemetry_scope"),
        Some(&json!("best_effort_approx_process_rss_step_delta_via_ps"))
    );
    assert_eq!(
        report.get("benchmark_temp_root"),
        Some(&json!(temp_root.display().to_string()))
    );
    assert_eq!(
        report.get("benchmark_temp_root_source"),
        Some(&json!("explicit"))
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("summary_steady_state"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("speedup_ratio_p95"))
            .is_some()
    );
    assert!(report.get("window_only_latency_ms").is_some());
    assert!(report.get("summary_window_cover_latency_ms").is_some());
    assert!(report.get("summary_rebuild_latency_ms").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_latency_ms")
            .is_some()
    );
    assert!(report.get("summary_metadata_realign_latency_ms").is_some());
    assert!(report.get("summary_steady_state_latency_ms").is_some());
    assert!(report.get("window_shrink_catch_up_latency_ms").is_some());
    assert!(
        report
            .get("window_only_append_pre_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("window_only_append_cold_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_pre_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_cold_overflow_latency_ms")
            .is_some()
    );
    assert!(report.get("summary_append_saturated_latency_ms").is_some());
    assert!(report.get("window_only_rss_delta_kib").is_some());
    assert!(report.get("summary_window_cover_rss_delta_kib").is_some());
    assert!(report.get("summary_rebuild_rss_delta_kib").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_metadata_realign_rss_delta_kib")
            .is_some()
    );
    assert!(report.get("summary_steady_state_rss_delta_kib").is_some());
    assert!(report.get("window_shrink_catch_up_rss_delta_kib").is_some());
    assert!(
        report
            .get("window_only_append_pre_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("window_only_append_cold_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_pre_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_cold_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_saturated_rss_delta_kib")
            .is_some()
    );
    let prompt_frame_stability = report
        .get("prompt_frame_stability")
        .expect("prompt frame stability section");
    assert_eq!(
        prompt_frame_stability.get("representative_scenario"),
        Some(&json!("summary_steady_state"))
    );
    assert_eq!(
        prompt_frame_stability.get("stable_prefix_preserved_on_followup"),
        Some(&json!(true))
    );
    assert_eq!(
        prompt_frame_stability.get("cached_prefix_preserved_on_followup"),
        Some(&json!(true))
    );
    assert_eq!(
        prompt_frame_stability.get("turn_ephemeral_hash_changed_on_followup"),
        Some(&json!(true))
    );
    assert!(
        prompt_frame_stability
            .get("layer_estimated_tokens")
            .and_then(|value| value.get("stable_prefix"))
            .is_some()
    );
    assert!(
        prompt_frame_stability
            .get("layer_estimated_tokens")
            .and_then(|value| value.get("followup_turn_ephemeral"))
            .is_some()
    );
    assert!(report.get("window_only_payload_chars").is_some());
    assert!(report.get("summary_window_cover_payload_chars").is_some());
    assert!(report.get("summary_rebuild_payload_chars").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_payload_chars")
            .is_some()
    );
    assert!(
        report
            .get("summary_metadata_realign_payload_chars")
            .is_some()
    );
    assert!(report.get("summary_steady_state_payload_chars").is_some());
    assert!(report.get("window_shrink_catch_up_payload_chars").is_some());
    assert!(
        report
            .get("prompt_efficiency_signals")
            .and_then(|value| value.get("summary_rebuild_budget_change"))
            .and_then(|value| value.get("estimated_session_local_recall_chars"))
            .is_some()
    );
    assert!(
        report
            .get("prompt_efficiency_signals")
            .and_then(|value| value.get("summary_metadata_realign"))
            .and_then(|value| value.get("estimated_non_recall_context_chars"))
            .is_some()
    );
    assert!(
        report
            .get("prompt_efficiency_signals")
            .and_then(|value| value.get("summary_steady_state"))
            .and_then(|value| value.get("estimated_session_local_recall_chars"))
            .is_some()
    );
    assert!(
        report
            .get("prompt_efficiency_signals")
            .and_then(|value| value.get("summary_steady_state"))
            .and_then(|value| value.get("estimated_non_recall_context_chars"))
            .is_some()
    );
    assert!(
        report
            .get("prompt_efficiency_signals")
            .and_then(|value| value.get("summary_steady_state"))
            .and_then(|value| value.get("estimated_session_local_recall_share_ratio"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_window_cover_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_window_cover_overhead_p95_ms"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_rebuild_budget_change_vs_rebuild_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| {
                v.get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            })
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_metadata_realign_vs_budget_change_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("speedup_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("window_shrink_catch_up_vs_rebuild_speedup_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_append_pre_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("window_only_append_pre_overflow"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("window_only_append_cold_overflow"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("summary_append_pre_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_max_overhead_p95_ms"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_warning_min_samples"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_rebuild_budget_change_vs_rebuild_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_metadata_realign_vs_budget_change_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("suite_stability_soft_warning_min_suites"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("suite_stability_soft_max_range_over_p50"))
            .is_some()
    );
    assert!(report.get("gate").and_then(|g| g.get("warnings")).is_some());
    let summary_window_cover_entry_count = report
        .get("summary_window_cover_entry_count")
        .and_then(Value::as_u64)
        .expect("summary_window_cover_entry_count should be present");
    let summary_window_cover_turn_entries = report
        .get("summary_window_cover_turn_entries")
        .and_then(Value::as_u64)
        .expect("summary_window_cover_turn_entries should be present");
    assert!(
        summary_window_cover_entry_count == summary_window_cover_turn_entries,
        "summary_window_cover should stay on the window-only surface until the history overflows"
    );

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}
