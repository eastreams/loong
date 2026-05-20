use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::{
    benchmark_temp_root, resolve_memory_context_benchmark_temp_root,
    resolve_memory_context_benchmark_temp_root_with_exe,
};
use super::*;

#[derive(Debug, Clone)]
struct PromptContextReadObservation {
    latency_ms: f64,
    rss_delta_kib: Option<f64>,
    shape: MemoryContextShape,
}

fn measure_hot_prompt_context_reads_with_loader(
    warmup_iterations: usize,
    hot_iterations: usize,
    expect_summary: bool,
    mut load_observation: impl FnMut() -> CliResult<PromptContextReadObservation>,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    for _ in 0..warmup_iterations.max(1) {
        let observation = load_observation()?;
        validate_prompt_context_shape(observation.shape, expect_summary, "warmup")?;
    }

    let mut latencies = Vec::with_capacity(hot_iterations);
    let mut rss_deltas_kib = Vec::with_capacity(hot_iterations);
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };

    for _ in 0..hot_iterations {
        let observation = load_observation()?;
        latencies.push(observation.latency_ms);
        if let Some(delta_kib) = observation.rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
        validate_prompt_context_shape(observation.shape, expect_summary, "sample")?;
        final_shape = observation.shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape))
}

fn validate_prompt_context_shape(
    shape: MemoryContextShape,
    expect_summary: bool,
    phase: &str,
) -> CliResult<()> {
    if expect_summary && shape.summary_chars == 0 {
        return Err(format!(
            "summary benchmark {phase} did not produce a summary entry"
        ));
    }
    if !expect_summary && shape.summary_chars != 0 {
        return Err(format!(
            "window-only benchmark {phase} unexpectedly produced a summary entry"
        ));
    }
    Ok(())
}

fn parse_ps_rss_kib_output(raw: &str) -> Option<f64> {
    let token = raw.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.split_whitespace().next()
        }
    })?;
    token.parse::<f64>().ok()
}

fn compute_rss_step_delta_kib(baseline_kib: Option<f64>, current_kib: Option<f64>) -> Option<f64> {
    let baseline_kib = baseline_kib?;
    let current_kib = current_kib?;
    Some((current_kib - baseline_kib).max(0.0))
}

#[test]
fn parse_ps_rss_kib_output_extracts_first_non_empty_numeric_value() {
    assert_eq!(parse_ps_rss_kib_output("  12345\n"), Some(12_345.0));
    assert_eq!(parse_ps_rss_kib_output("\n  6789 extra\n"), Some(6_789.0));
}

#[test]
fn parse_ps_rss_kib_output_rejects_blank_or_invalid_values() {
    assert_eq!(parse_ps_rss_kib_output(""), None);
    assert_eq!(parse_ps_rss_kib_output("  \n"), None);
    assert_eq!(parse_ps_rss_kib_output("rss\n"), None);
}

#[test]
fn compute_rss_step_delta_kib_clamps_negative_and_propagates_missing_samples() {
    assert_eq!(
        compute_rss_step_delta_kib(Some(100.0), Some(112.0)),
        Some(12.0)
    );
    assert_eq!(
        compute_rss_step_delta_kib(Some(112.0), Some(100.0)),
        Some(0.0)
    );
    assert_eq!(compute_rss_step_delta_kib(None, Some(100.0)), None);
    assert_eq!(compute_rss_step_delta_kib(Some(100.0), None), None);
}

#[test]
fn format_optional_decimal_returns_na_when_value_missing() {
    assert_eq!(format_optional_decimal(Some(12.34), 1), "12.3");
    assert_eq!(format_optional_decimal(None, 1), "n/a");
}

#[test]
fn memory_context_soft_warnings_ignore_cover_path_noise_floor() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert!(warnings.is_empty());
}

#[test]
fn memory_context_soft_warnings_flag_cover_path_regression_beyond_noise_floor() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.24),
        Some(0.083),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("summary_window_cover"));
    assert!(warnings[0].contains("soft thresholds"));
    assert!(!warnings[0].contains("suite-noisy"));
}

#[test]
fn memory_context_soft_warnings_ignore_marginal_cover_regression_when_suite_is_noisy() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.177),
        Some(0.150),
        16,
        true,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        Some(1.496),
        Some(1.897),
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert!(
        warnings
            .iter()
            .all(|warning| !warning.contains("summary_window_cover")),
        "expected marginal cover-path regressions to stay silent when the surrounding suite is already too noisy for path-specific attribution"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("speedup_ratio_p95")),
        "expected suite-noise warnings to remain visible when cover-path specificity is suppressed"
    );
}

#[test]
fn memory_context_soft_warnings_keep_large_cover_regression_even_when_suite_is_noisy() {
    let warnings = build_memory_context_soft_warnings(
        Some(3.012),
        Some(2.074),
        16,
        true,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        Some(2.670),
        Some(2.773),
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("summary_window_cover")),
        "expected clearly excessive cover-path regressions to keep their dedicated warning even on a noisy host"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("summary_window_cover")
                && warning.contains("suite-noisy")),
        "expected non-marginal cover regressions on noisy suites to stay visible but be qualified as suite-noisy"
    );
}

#[test]
fn memory_context_soft_warnings_flag_budget_change_path_regression() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(1.12),
        Some(1.12),
        16,
        Some(0.91),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("summary_rebuild_budget_change"));
    assert!(warnings[0].contains("full rebuild"));
}

#[test]
fn memory_context_soft_warnings_ignore_budget_change_when_workload_adjusted_ratio_is_stable() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(1.12),
        Some(0.58),
        16,
        Some(0.91),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert!(
        warnings
            .iter()
            .all(|warning| !warning.contains("summary_rebuild_budget_change")),
        "expected budget-change warnings to stay quiet when a larger rebuilt summary fully explains the raw latency ratio"
    );
}

#[test]
fn memory_context_soft_warnings_flag_metadata_realign_regression() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(1.18),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("summary_metadata_realign"));
    assert!(warnings[0].contains("budget-change rebuild"));
    assert!(!warnings[0].contains("suite-noisy"));
}

#[test]
fn memory_context_soft_warnings_require_stable_sample_size() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.24),
        Some(0.083),
        4,
        false,
        Some(1.12),
        Some(1.12),
        4,
        Some(1.18),
        4,
        None,
        None,
        None,
        None,
        None,
        None,
        2,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );
    assert!(warnings.is_empty());
}

#[test]
fn memory_context_soft_warnings_flag_system_temp_root_fallback() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        None,
        None,
        None,
        None,
        None,
        5,
        1.10,
        None,
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::SystemTemp,
        Path::new("/tmp"),
    );

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("benchmark_temp_root"));
    assert!(warnings[0].contains("system temp"));
}

#[test]
fn memory_context_soft_warnings_keep_speedup_warning_generic_without_rebuild_noise() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        Some(0.84),
        Some(0.19),
        None,
        None,
        None,
        5,
        1.10,
        Some(&MemoryContextColdPathNoiseAttribution {
            phase: "copy_db_ms".to_owned(),
            range_over_p50: 0.19,
        }),
        None,
        None,
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("speedup_ratio_p95"));
    assert!(!warnings[0].contains("dominant summary_rebuild cold-path noise"));
}

#[test]
fn memory_context_soft_warnings_expand_target_load_noise_subphase_labels() {
    let warnings = build_memory_context_soft_warnings(
        Some(1.04),
        Some(0.012),
        16,
        false,
        Some(0.82),
        Some(0.82),
        16,
        Some(0.91),
        16,
        None,
        Some(0.84),
        Some(0.91),
        None,
        None,
        None,
        5,
        1.10,
        Some(&MemoryContextColdPathNoiseAttribution {
            phase: "target_load_ms".to_owned(),
            range_over_p50: 0.91,
        }),
        None,
        Some(&MemoryContextLoadNoiseAttribution {
            phase: "summary_catch_up_ms".to_owned(),
            range_over_p50: 0.88,
        }),
        None,
        None,
        None,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        Path::new("target/codex-memory-bench-red/tmp-local"),
    );

    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().any(|warning| {
        warning.contains("summary_rebuild suite p95")
            && warning.contains("target_load_ms/summary_catch_up_ms")
    }));
}

#[test]
fn memory_context_benchmark_report_emits_prompt_efficiency_signals() {
    let window_only_shape = MemoryContextShape {
        entry_count: 4,
        turn_entries: 4,
        summary_chars: 0,
        payload_chars: 400,
    };
    let summary_window_cover_shape = MemoryContextShape {
        entry_count: 5,
        turn_entries: 4,
        summary_chars: 80,
        payload_chars: 480,
    };
    let summary_rebuild_shape = MemoryContextShape {
        entry_count: 6,
        turn_entries: 4,
        summary_chars: 120,
        payload_chars: 520,
    };
    let summary_steady_state_shape = MemoryContextShape {
        entry_count: 6,
        turn_entries: 4,
        summary_chars: 100,
        payload_chars: 420,
    };
    let window_shrink_catch_up_shape = MemoryContextShape {
        entry_count: 5,
        turn_entries: 3,
        summary_chars: 90,
        payload_chars: 390,
    };
    let suite_runs = vec![MemoryContextBenchmarkSuiteSamples {
        seed_db_bytes: 1024,
        window_only_samples: vec![1.0, 1.2],
        summary_window_cover_samples: vec![1.05, 1.25],
        summary_rebuild_samples: vec![2.0, 2.2],
        summary_rebuild_budget_change_samples: vec![1.3, 1.4],
        summary_metadata_realign_samples: vec![1.2, 1.25],
        summary_steady_state_samples: vec![0.7, 0.75],
        window_shrink_catch_up_samples: vec![0.9, 0.95],
        window_only_append_pre_overflow_samples: vec![0.8, 0.82],
        window_only_append_cold_overflow_samples: vec![0.85, 0.9],
        summary_append_pre_overflow_samples: vec![1.1, 1.15],
        summary_append_cold_overflow_samples: vec![1.4, 1.5],
        summary_append_saturated_samples: vec![1.0, 1.05],
        window_only_rss_deltas_kib: vec![0.0, 16.0],
        summary_window_cover_rss_deltas_kib: vec![0.0, 16.0],
        summary_rebuild_rss_deltas_kib: vec![32.0, 48.0],
        summary_rebuild_budget_change_rss_deltas_kib: vec![16.0, 32.0],
        summary_metadata_realign_rss_deltas_kib: vec![16.0, 16.0],
        summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
        window_shrink_catch_up_rss_deltas_kib: vec![16.0, 16.0],
        window_only_append_pre_overflow_rss_deltas_kib: vec![16.0, 16.0],
        window_only_append_cold_overflow_rss_deltas_kib: vec![16.0, 32.0],
        summary_append_pre_overflow_rss_deltas_kib: vec![16.0, 32.0],
        summary_append_cold_overflow_rss_deltas_kib: vec![32.0, 32.0],
        summary_append_saturated_rss_deltas_kib: vec![16.0, 16.0],
        summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_only_shape,
        summary_window_cover_shape,
        summary_rebuild_shape,
        summary_rebuild_budget_change_shape: summary_rebuild_shape,
        summary_metadata_realign_shape: summary_rebuild_shape,
        summary_steady_state_shape,
        window_shrink_catch_up_shape,
    }];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        1,
        false,
        1.10,
    );

    let window_only_signal = &report.prompt_efficiency_signals.window_only;
    let steady_state_signal = &report.prompt_efficiency_signals.summary_steady_state;
    let rebuild_signal = &report.prompt_efficiency_signals.summary_rebuild;
    let budget_change_signal = &report
        .prompt_efficiency_signals
        .summary_rebuild_budget_change;
    let metadata_realign_signal = &report.prompt_efficiency_signals.summary_metadata_realign;

    assert_eq!(window_only_signal.estimated_session_local_recall_chars, 0);
    assert_eq!(
        steady_state_signal.estimated_session_local_recall_chars,
        100
    );
    assert_eq!(steady_state_signal.estimated_non_recall_context_chars, 320);
    assert_eq!(rebuild_signal.estimated_session_local_recall_chars, 120);
    assert_eq!(
        budget_change_signal.estimated_session_local_recall_chars,
        120
    );
    assert_eq!(
        metadata_realign_signal.estimated_session_local_recall_chars,
        120
    );
    assert_eq!(
        metadata_realign_signal.estimated_non_recall_context_chars,
        400
    );

    let rebuild_share_ratio = rebuild_signal
        .estimated_session_local_recall_share_ratio
        .expect("rebuild recall share ratio");
    let expected_rebuild_share_ratio = 120.0 / 520.0;
    let delta = (rebuild_share_ratio - expected_rebuild_share_ratio).abs();

    assert!(delta < 1e-9);
}

#[test]
fn memory_context_benchmark_report_tracks_append_window_only_baselines() {
    let shape = MemoryContextShape {
        entry_count: 7,
        turn_entries: 6,
        summary_chars: 256,
        payload_chars: 768,
    };
    let suite_runs = vec![
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 1024,
            window_only_samples: vec![1.0, 1.2],
            summary_window_cover_samples: vec![1.05, 1.25],
            summary_rebuild_samples: vec![2.0, 2.2],
            summary_rebuild_budget_change_samples: vec![1.3, 1.4],
            summary_metadata_realign_samples: vec![1.2, 1.25],
            summary_steady_state_samples: vec![0.7, 0.75],
            window_shrink_catch_up_samples: vec![0.9, 0.95],
            window_only_append_pre_overflow_samples: vec![0.8, 0.82],
            window_only_append_cold_overflow_samples: vec![0.85, 0.9],
            summary_append_pre_overflow_samples: vec![1.1, 1.15],
            summary_append_cold_overflow_samples: vec![1.4, 1.5],
            summary_append_saturated_samples: vec![1.0, 1.05],
            window_only_rss_deltas_kib: vec![0.0, 16.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 16.0],
            summary_rebuild_rss_deltas_kib: vec![32.0, 48.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![16.0, 32.0],
            summary_metadata_realign_rss_deltas_kib: vec![16.0, 16.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![16.0, 16.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![16.0, 16.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![16.0, 32.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![16.0, 32.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![32.0, 32.0],
            summary_append_saturated_rss_deltas_kib: vec![16.0, 16.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 1024,
            window_only_samples: vec![1.1, 1.3],
            summary_window_cover_samples: vec![1.15, 1.35],
            summary_rebuild_samples: vec![2.1, 2.4],
            summary_rebuild_budget_change_samples: vec![1.35, 1.45],
            summary_metadata_realign_samples: vec![1.22, 1.28],
            summary_steady_state_samples: vec![0.72, 0.77],
            window_shrink_catch_up_samples: vec![0.92, 1.0],
            window_only_append_pre_overflow_samples: vec![0.82, 0.86],
            window_only_append_cold_overflow_samples: vec![0.9, 0.94],
            summary_append_pre_overflow_samples: vec![1.18, 1.22],
            summary_append_cold_overflow_samples: vec![1.48, 1.58],
            summary_append_saturated_samples: vec![1.02, 1.08],
            window_only_rss_deltas_kib: vec![0.0, 16.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 16.0],
            summary_rebuild_rss_deltas_kib: vec![32.0, 48.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![16.0, 32.0],
            summary_metadata_realign_rss_deltas_kib: vec![16.0, 16.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![16.0, 16.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![16.0, 16.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![16.0, 32.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![16.0, 32.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![32.0, 48.0],
            summary_append_saturated_rss_deltas_kib: vec![16.0, 16.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        2,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert!(
        report_json
            .get("window_only_append_pre_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report_json
            .get("window_only_append_cold_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report_json
            .get("window_only_append_pre_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report_json
            .get("window_only_append_cold_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report_json
            .get("flattened_sample_ratios")
            .and_then(|value| {
                value
                    .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            })
            .is_some()
    );
    assert!(
        report_json
            .get("aggregated_ratios")
            .and_then(|value| {
                value
                    .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            })
            .is_some()
    );
    assert!(
        report_json
            .get("flattened_sample_ratios")
            .and_then(|value| { value.get("summary_append_pre_overflow_vs_window_only_ratio_p95") })
            .is_some()
    );
    assert!(
        report_json
            .get("flattened_sample_ratios")
            .and_then(|value| {
                value.get("summary_append_cold_overflow_vs_window_only_ratio_p95")
            })
            .is_some()
    );
    assert!(
        report_json
            .get("aggregated_p95_median_ms")
            .and_then(|value| value.get("window_only_append_pre_overflow"))
            .is_some()
    );
    assert!(
        report_json
            .get("aggregated_p95_median_ms")
            .and_then(|value| value.get("window_only_append_cold_overflow"))
            .is_some()
    );
    assert!(
        report_json
            .get("aggregated_ratios")
            .and_then(|value| { value.get("summary_append_pre_overflow_vs_window_only_ratio_p95") })
            .is_some()
    );
    assert!(
        report_json
            .get("aggregated_ratios")
            .and_then(|value| {
                value.get("summary_append_cold_overflow_vs_window_only_ratio_p95")
            })
            .is_some()
    );
}

#[test]
fn memory_context_benchmark_report_separates_flattened_and_aggregated_ratio_views() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let suite_runs = vec![
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![10.0, 10.0],
            summary_rebuild_budget_change_samples: vec![5.0, 5.0],
            summary_metadata_realign_samples: vec![2.0, 2.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![10.0, 10.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![11.0, 11.0],
            summary_rebuild_budget_change_samples: vec![5.5, 5.5],
            summary_metadata_realign_samples: vec![2.2, 2.2],
            summary_steady_state_samples: vec![10.0, 10.0],
            window_shrink_catch_up_samples: vec![9.0, 9.0],
            window_only_append_pre_overflow_samples: vec![10.0, 10.0],
            window_only_append_cold_overflow_samples: vec![10.0, 10.0],
            summary_append_pre_overflow_samples: vec![10.0, 10.0],
            summary_append_cold_overflow_samples: vec![11.0, 11.0],
            summary_append_saturated_samples: vec![10.0, 10.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        2,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    let flattened_append_cold_ratio = report_json
        .get("flattened_sample_ratios")
        .and_then(|value| value.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
        .and_then(Value::as_f64)
        .expect("flattened append-cold ratio should be present");
    let aggregated_append_cold_ratio = report_json
        .get("aggregated_ratios")
        .and_then(|value| value.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
        .and_then(Value::as_f64)
        .expect("aggregated append-cold ratio should be present");

    assert!(
        report_json
            .get("summary_append_cold_overflow_vs_window_only_ratio_p95")
            .is_none(),
        "expected the report root to stop exposing ambiguous ratio fields once flattened_sample_ratios is available"
    );
    assert!(
        (flattened_append_cold_ratio - aggregated_append_cold_ratio).abs() > 1.0,
        "expected the fixture to preserve a visible difference between flattened-sample and aggregated ratio views"
    );
}

#[test]
fn memory_context_benchmark_report_uses_aggregated_ratio_view_for_soft_warnings() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |cover_samples: Vec<f64>,
                      budget_samples: Vec<f64>,
                      metadata_samples: Vec<f64>| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0, 1.0, 1.0],
            summary_window_cover_samples: cover_samples,
            summary_rebuild_samples: vec![4.0, 4.0, 4.0, 4.0],
            summary_rebuild_budget_change_samples: budget_samples,
            summary_metadata_realign_samples: metadata_samples,
            summary_steady_state_samples: vec![1.0, 1.0, 1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0, 2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0, 1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(
            vec![1.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.0, 1.0, 1.0],
        ),
        make_suite(
            vec![1.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.0, 1.0, 1.0],
        ),
        make_suite(
            vec![1.0, 1.0, 2.0, 2.0],
            vec![1.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.0, 2.0, 2.0],
        ),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert_eq!(
        report
            .flattened_sample_ratios
            .summary_window_cover_vs_window_only_ratio_p95,
        Some(2.0)
    );
    assert_eq!(
        report
            .aggregated_ratios
            .summary_window_cover_vs_window_only_ratio_p95,
        Some(1.0)
    );
    assert_eq!(
        report
            .flattened_sample_ratios
            .summary_metadata_realign_vs_budget_change_ratio_p95,
        Some(2.0)
    );
    assert_eq!(
        report
            .aggregated_ratios
            .summary_metadata_realign_vs_budget_change_ratio_p95,
        Some(1.0)
    );
    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("summary_window_cover")),
        "expected cover-path warnings to key off aggregated suite-median ratios instead of flattened tails"
    );
    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("summary_metadata_realign")),
        "expected metadata-realign warnings to key off aggregated suite-median ratios instead of flattened tails"
    );
}

#[test]
fn memory_context_benchmark_report_emits_suite_p95_summaries_for_noise_analysis() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let suite_runs = vec![
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.2],
            summary_window_cover_samples: vec![0.9, 1.1],
            summary_rebuild_samples: vec![2.0, 2.2],
            summary_rebuild_budget_change_samples: vec![1.0, 1.1],
            summary_metadata_realign_samples: vec![0.8, 0.9],
            summary_steady_state_samples: vec![0.5, 0.55],
            window_shrink_catch_up_samples: vec![0.7, 0.75],
            window_only_append_pre_overflow_samples: vec![0.8, 0.82],
            window_only_append_cold_overflow_samples: vec![0.9, 0.95],
            summary_append_pre_overflow_samples: vec![0.7, 0.74],
            summary_append_cold_overflow_samples: vec![1.1, 1.2],
            summary_append_saturated_samples: vec![0.6, 0.65],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![2.0, 2.2],
            summary_window_cover_samples: vec![2.1, 2.3],
            summary_rebuild_samples: vec![3.0, 3.2],
            summary_rebuild_budget_change_samples: vec![1.5, 1.7],
            summary_metadata_realign_samples: vec![1.2, 1.3],
            summary_steady_state_samples: vec![0.9, 1.0],
            window_shrink_catch_up_samples: vec![1.1, 1.2],
            window_only_append_pre_overflow_samples: vec![1.3, 1.4],
            window_only_append_cold_overflow_samples: vec![1.4, 1.5],
            summary_append_pre_overflow_samples: vec![1.0, 1.05],
            summary_append_cold_overflow_samples: vec![1.8, 1.9],
            summary_append_saturated_samples: vec![0.9, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        },
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        2,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    let suite_summaries = report_json
        .get("suite_p95_summaries")
        .and_then(Value::as_array)
        .expect("suite p95 summaries should be present");
    assert_eq!(suite_summaries.len(), 2);
    assert!(
        suite_summaries[0]
            .get("summary_append_cold_overflow")
            .and_then(Value::as_f64)
            .is_some(),
        "expected each suite summary to expose scenario-level p95s for direct noise inspection"
    );
    assert!(
        suite_summaries[0]
            .get("summary_append_cold_overflow_vs_window_only_ratio_p95")
            .and_then(Value::as_f64)
            .is_some(),
        "expected each suite summary to expose ratio-level p95s for direct noise inspection"
    );
    assert!(
        suite_summaries[0]
            .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            .and_then(Value::as_f64)
            .is_some(),
        "expected each suite summary to expose workload-adjusted budget-change ratios for direct noise inspection"
    );
}

#[test]
fn memory_context_benchmark_report_emits_suite_stability_summary() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |window_only: f64,
         summary_window_cover: f64,
         summary_rebuild: f64,
         summary_rebuild_budget_change: f64,
         summary_metadata_realign: f64,
         summary_steady_state: f64,
         window_shrink_catch_up: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![window_only, window_only],
            summary_window_cover_samples: vec![summary_window_cover, summary_window_cover],
            summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
            summary_rebuild_budget_change_samples: vec![
                summary_rebuild_budget_change,
                summary_rebuild_budget_change,
            ],
            summary_metadata_realign_samples: vec![
                summary_metadata_realign,
                summary_metadata_realign,
            ],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![window_shrink_catch_up, window_shrink_catch_up],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(1.0, 0.8, 4.0, 2.0, 1.0, 1.0, 2.0),
        make_suite(2.0, 1.6, 5.0, 2.5, 1.5, 1.25, 2.5),
        make_suite(3.0, 2.4, 6.0, 3.0, 2.0, 1.5, 3.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("suite_stability")
            .and_then(|value| value.get("window_only_p95_ms"))
            .and_then(|value| value.get("count"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        report_json
            .get("suite_stability")
            .and_then(|value| value.get("window_only_p95_ms"))
            .and_then(|value| value.get("range"))
            .and_then(Value::as_f64),
        Some(2.0)
    );
    assert_eq!(
        report_json
            .get("suite_stability")
            .and_then(|value| value.get("summary_window_cover_vs_window_only_ratio_p95"))
            .and_then(|value| value.get("range"))
            .and_then(Value::as_f64),
        Some(0.0)
    );
    assert_eq!(
        report_json
            .get("suite_stability")
            .and_then(|value| value.get("speedup_ratio_p95"))
            .and_then(|value| value.get("max_over_p50"))
            .and_then(Value::as_f64),
        Some(1.0)
    );
}

#[test]
fn memory_context_benchmark_report_warns_when_speedup_ratio_is_suite_noisy() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
        seed_db_bytes: 512,
        window_only_samples: vec![1.0, 1.0],
        summary_window_cover_samples: vec![1.0, 1.0],
        summary_rebuild_samples: vec![4.0, 4.0],
        summary_rebuild_budget_change_samples: vec![2.0, 2.0],
        summary_metadata_realign_samples: vec![1.0, 1.0],
        summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
        window_shrink_catch_up_samples: vec![2.0, 2.0],
        window_only_append_pre_overflow_samples: vec![1.0, 1.0],
        window_only_append_cold_overflow_samples: vec![1.0, 1.0],
        summary_append_pre_overflow_samples: vec![1.0, 1.0],
        summary_append_cold_overflow_samples: vec![1.0, 1.0],
        summary_append_saturated_samples: vec![1.0, 1.0],
        window_only_rss_deltas_kib: vec![0.0, 0.0],
        summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
        summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
        summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
        window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
        window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
        window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_only_shape: shape,
        summary_window_cover_shape: shape,
        summary_rebuild_shape: shape,
        summary_rebuild_budget_change_shape: shape,
        summary_metadata_realign_shape: shape,
        summary_steady_state_shape: shape,
        window_shrink_catch_up_shape: shape,
    };
    let suite_runs = vec![make_suite(1.0), make_suite(2.0), make_suite(4.0)];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report
            .gate
            .warnings
            .iter()
            .any(|warning| warning.contains("speedup_ratio_p95")),
        "expected suite stability warning for noisy speedup ratio"
    );
}

#[test]
fn memory_context_benchmark_report_qualifies_cover_warning_under_suite_noise() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |window_only: f64, summary_window_cover: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![window_only, window_only],
            summary_window_cover_samples: vec![summary_window_cover, summary_window_cover],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(0.4300038, 0.31504375),
        make_suite(0.44856565, 1.1060792),
        make_suite(0.8337033, 0.45344445),
        make_suite(0.9723356, 1.72131875),
        make_suite(0.6513814, 1.0745410),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    let cover_warning = report
        .gate
        .warnings
        .iter()
        .find(|warning| warning.contains("summary_window_cover"))
        .expect("expected cover warning");
    assert!(
        cover_warning.contains("suite-noisy"),
        "expected noisy cover-path comparisons to be qualified as suite-noisy instead of being presented as a direct product regression"
    );
    assert!(
        !cover_warning.contains("redundant summary materialization or checkpoint work"),
        "expected suite-noisy cover warnings to avoid over-specific product-cause guidance"
    );
}

#[test]
fn memory_context_benchmark_report_qualifies_metadata_realign_warning_under_suite_noise() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |summary_rebuild_budget_change: f64, summary_metadata_realign: f64| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![
                summary_rebuild_budget_change,
                summary_rebuild_budget_change,
            ],
            summary_metadata_realign_samples: vec![
                summary_metadata_realign,
                summary_metadata_realign,
            ],
            summary_steady_state_samples: vec![2.0, 2.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(1.36408355, 0.31361875),
        make_suite(0.45087515, 3.65480625),
        make_suite(0.65107080, 1.56425160),
        make_suite(0.49981005, 0.98310625),
        make_suite(0.70441715, 0.34252745),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    let metadata_warning = report
        .gate
        .warnings
        .iter()
        .find(|warning| warning.contains("summary_metadata_realign"))
        .expect("expected metadata-realign warning");
    assert!(
        metadata_warning.contains("suite-noisy"),
        "expected noisy metadata/budget-change comparisons to be qualified as suite-noisy instead of being presented as a direct product regression"
    );
    assert!(
        !metadata_warning.contains("accidental summary body rewrites"),
        "expected suite-noisy metadata warning to avoid over-specific product-cause guidance"
    );
}

#[test]
fn memory_context_benchmark_report_suppresses_speedup_warning_when_copy_noise_dominates_clear_wins()
{
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |summary_rebuild: f64, summary_steady_state: f64, copy_db_ms: f64, target_load_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![copy_db_ms, copy_db_ms],
                    target_load_ms: vec![target_load_ms, target_load_ms],
                    target_load_summary_rebuild_ms: vec![target_load_ms, target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
    let suite_runs = vec![
        make_suite(4.0, 0.40, 1.0, 3.0),
        make_suite(8.0, 1.60, 5.0, 8.0),
        make_suite(12.0, 3.00, 9.0, 12.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("speedup_ratio_p95")),
        "expected non-marginal speedup wins to ignore suite-noise warnings when copy_db_ms is the dominant rebuild-noise source"
    );
    assert!(
        report
            .gate
            .warnings
            .iter()
            .any(|warning| warning.contains("summary_rebuild suite p95")),
        "expected summary_rebuild instability warning to remain visible for the underlying cold-path noise"
    );
}

#[test]
fn memory_context_benchmark_report_suppresses_speedup_warning_when_bootstrap_noise_dominates_clear_wins()
 {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |summary_rebuild: f64,
                      summary_steady_state: f64,
                      target_bootstrap_ms: f64,
                      schema_upgrade_ms: f64,
                      target_load_ms: f64| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                target_bootstrap_ms: vec![target_bootstrap_ms, target_bootstrap_ms],
                target_bootstrap_schema_upgrade_ms: vec![schema_upgrade_ms, schema_upgrade_ms],
                target_load_ms: vec![target_load_ms, target_load_ms],
                target_load_summary_rebuild_ms: vec![target_load_ms, target_load_ms],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(4.0, 0.20, 1.0, 1.0, 3.0),
        make_suite(8.0, 0.80, 8.0, 8.0, 4.0),
        make_suite(12.0, 1.20, 16.0, 16.0, 5.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("speedup_ratio_p95")),
        "expected non-marginal speedup wins to ignore suite-noise warnings when bootstrap noise dominates rebuild instability"
    );
    assert!(
        report
            .gate
            .warnings
            .iter()
            .any(|warning| warning.contains("summary_rebuild suite p95")),
        "expected summary_rebuild instability warning to remain visible when bootstrap noise dominates"
    );
}

#[test]
fn memory_context_benchmark_report_suppresses_speedup_warning_for_tiny_hot_path_denominator_jitter()
{
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(3.5, 0.30),
        make_suite(3.8, 0.60),
        make_suite(4.2, 1.20),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("speedup_ratio_p95")),
        "expected clear speedup wins to ignore speedup-ratio suite noise when only a tiny hot-path denominator jitter is inflating the ratio spread"
    );
    assert!(
        report
            .gate
            .warnings
            .iter()
            .all(|warning| !warning.contains("summary_rebuild suite p95")),
        "expected summary_rebuild to stay classified as stable in the hot-denominator jitter case"
    );
}

#[test]
fn memory_context_benchmark_report_keeps_speedup_warning_when_hot_path_spread_is_not_tiny() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(12.0, 2.0),
        make_suite(12.6, 4.0),
        make_suite(13.2, 6.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report
            .gate
            .warnings
            .iter()
            .any(|warning| warning.contains("speedup_ratio_p95")),
        "expected speedup-ratio suite warning to remain visible once hot-path absolute spread is large enough to be operationally meaningful"
    );
}

#[test]
fn memory_context_benchmark_report_qualifies_speedup_warning_when_all_suites_still_clear_the_floor()
{
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &[
            make_suite(12.0, 2.0),
            make_suite(12.2, 4.0),
            make_suite(12.4, 6.0),
        ],
        3,
        false,
        1.10,
    );

    let speedup_warning = report
        .gate
        .warnings
        .iter()
        .find(|warning| warning.contains("speedup_ratio_p95"))
        .expect("expected speedup warning to remain visible for suite noise");
    assert!(
        speedup_warning.contains("every suite still cleared the speedup floor"),
        "expected clear-win suite noise to be qualified instead of described as marginal"
    );
    assert!(
        !speedup_warning.contains("marginal memory context improvements"),
        "expected clear-win suite noise wording to avoid marginal-gain guidance"
    );
}

#[test]
fn memory_context_benchmark_report_warns_when_summary_rebuild_is_suite_noisy() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |summary_rebuild: f64,
                      summary_steady_state: f64,
                      target_bootstrap_ms: f64,
                      target_bootstrap_connection_open_ms: f64,
                      target_load_ms: f64,
                      copy_db_ms: f64| MemoryContextBenchmarkSuiteSamples {
        seed_db_bytes: 512,
        window_only_samples: vec![1.0, 1.0],
        summary_window_cover_samples: vec![1.0, 1.0],
        summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
        summary_rebuild_budget_change_samples: vec![2.0, 2.0],
        summary_metadata_realign_samples: vec![1.0, 1.0],
        summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
        window_shrink_catch_up_samples: vec![2.0, 2.0],
        window_only_append_pre_overflow_samples: vec![1.0, 1.0],
        window_only_append_cold_overflow_samples: vec![1.0, 1.0],
        summary_append_pre_overflow_samples: vec![1.0, 1.0],
        summary_append_cold_overflow_samples: vec![1.0, 1.0],
        summary_append_saturated_samples: vec![1.0, 1.0],
        window_only_rss_deltas_kib: vec![0.0, 0.0],
        summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
        summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
        summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
        window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
        window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
        window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
        summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
        summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
            copy_db_ms: vec![copy_db_ms, copy_db_ms],
            target_bootstrap_ms: vec![target_bootstrap_ms, target_bootstrap_ms],
            target_bootstrap_connection_open_ms: vec![
                target_bootstrap_connection_open_ms,
                target_bootstrap_connection_open_ms,
            ],
            target_load_ms: vec![target_load_ms, target_load_ms],
            ..MemoryContextColdPathPhaseSamples::default()
        },
        summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
        window_only_shape: shape,
        summary_window_cover_shape: shape,
        summary_rebuild_shape: shape,
        summary_rebuild_budget_change_shape: shape,
        summary_metadata_realign_shape: shape,
        summary_steady_state_shape: shape,
        window_shrink_catch_up_shape: shape,
    };
    let suite_runs = vec![
        make_suite(4.0, 1.0, 4.0, 4.0, 1.0, 1.0),
        make_suite(8.0, 2.0, 8.0, 8.0, 1.2, 1.2),
        make_suite(12.0, 3.0, 12.0, 12.0, 1.4, 1.4),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );

    assert!(
        report.gate.warnings.iter().any(|warning| {
            warning.contains("summary_rebuild suite p95")
                && warning.contains("target_bootstrap_ms/connection_open_ms")
        }),
        "expected suite stability warning for noisy summary_rebuild p95"
    );
}

#[test]
fn memory_context_benchmark_report_emits_cold_path_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |rebuild_target_load_ms: f64,
         rebuild_copy_db_ms: f64,
         budget_target_load_ms: f64,
         budget_source_warmup_ms: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                copy_db_ms: vec![rebuild_copy_db_ms, rebuild_copy_db_ms],
                target_load_ms: vec![rebuild_target_load_ms, rebuild_target_load_ms],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                source_warmup_ms: vec![budget_source_warmup_ms, budget_source_warmup_ms],
                target_load_ms: vec![budget_target_load_ms, budget_target_load_ms],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(4.0, 1.0, 2.0, 6.0),
        make_suite(8.0, 1.2, 2.5, 10.0),
        make_suite(12.0, 1.4, 3.0, 14.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_noise_attribution")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("target_load_ms")
    );
    assert_eq!(
        report_json
            .get("cold_path_noise_attribution")
            .and_then(|value| value.get("summary_rebuild_budget_change"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("source_warmup_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_cold_path_bootstrap_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |target_connection_open_ms: f64,
         target_schema_init_ms: f64,
         source_registry_lookup_ms: f64,
         source_schema_upgrade_ms: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                target_bootstrap_connection_open_ms: vec![
                    target_connection_open_ms,
                    target_connection_open_ms,
                ],
                target_bootstrap_schema_init_ms: vec![target_schema_init_ms, target_schema_init_ms],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                source_bootstrap_registry_lookup_ms: vec![
                    source_registry_lookup_ms,
                    source_registry_lookup_ms,
                ],
                source_bootstrap_schema_upgrade_ms: vec![
                    source_schema_upgrade_ms,
                    source_schema_upgrade_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let suite_runs = vec![
        make_suite(4.0, 1.0, 2.0, 6.0),
        make_suite(8.0, 1.2, 2.5, 10.0),
        make_suite(12.0, 1.4, 3.0, 14.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_bootstrap_noise_attribution")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_bootstrap"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("connection_open_ms")
    );
    assert_eq!(
        report_json
            .get("cold_path_bootstrap_noise_attribution")
            .and_then(|value| value.get("summary_rebuild_budget_change"))
            .and_then(|value| value.get("source_bootstrap"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("schema_upgrade_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_cold_path_load_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |budget_target_window_query_ms: f64,
                      budget_target_meta_query_ms: f64,
                      budget_target_update_returning_body_ms: f64,
                      metadata_target_window_query_ms: f64,
                      metadata_target_body_load_ms: f64,
                      metadata_target_catch_up_ms: f64| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                target_load_window_query_ms: vec![
                    budget_target_window_query_ms,
                    budget_target_window_query_ms,
                ],
                target_load_summary_checkpoint_meta_query_ms: vec![
                    budget_target_meta_query_ms,
                    budget_target_meta_query_ms,
                ],
                target_load_summary_checkpoint_metadata_update_returning_body_ms: vec![
                    budget_target_update_returning_body_ms,
                    budget_target_update_returning_body_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples {
                target_load_window_query_ms: vec![
                    metadata_target_window_query_ms,
                    metadata_target_window_query_ms,
                ],
                target_load_summary_checkpoint_body_load_ms: vec![
                    metadata_target_body_load_ms,
                    metadata_target_body_load_ms,
                ],
                target_load_summary_catch_up_ms: vec![
                    metadata_target_catch_up_ms,
                    metadata_target_catch_up_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(2.0, 3.0, 6.0, 1.0, 2.0, 5.0),
        make_suite(2.5, 3.5, 10.0, 1.2, 2.3, 9.0),
        make_suite(3.0, 4.0, 14.0, 1.4, 2.6, 13.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_load_noise_attribution")
            .and_then(|value| value.get("summary_rebuild_budget_change"))
            .and_then(|value| value.get("target_load"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("summary_checkpoint_metadata_update_returning_body_ms")
    );
    assert_eq!(
        report_json
            .get("cold_path_load_noise_attribution")
            .and_then(|value| value.get("summary_metadata_realign"))
            .and_then(|value| value.get("target_load"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("summary_catch_up_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_split_summary_rebuild_load_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |rebuild_stream_ms: f64,
                      rebuild_metadata_upsert_ms: f64,
                      rebuild_body_upsert_ms: f64,
                      rebuild_commit_ms: f64| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                target_load_summary_rebuild_ms: vec![1.0, 1.0],
                target_load_summary_rebuild_stream_ms: vec![rebuild_stream_ms, rebuild_stream_ms],
                target_load_summary_rebuild_checkpoint_metadata_upsert_ms: vec![
                    rebuild_metadata_upsert_ms,
                    rebuild_metadata_upsert_ms,
                ],
                target_load_summary_rebuild_checkpoint_body_upsert_ms: vec![
                    rebuild_body_upsert_ms,
                    rebuild_body_upsert_ms,
                ],
                target_load_summary_rebuild_checkpoint_commit_ms: vec![
                    rebuild_commit_ms,
                    rebuild_commit_ms,
                ],
                target_load_summary_rebuild_checkpoint_upsert_ms: vec![
                    rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                    rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(2.0, 0.3, 0.5, 0.4),
        make_suite(10.0, 0.4, 0.6, 0.5),
        make_suite(14.0, 0.5, 0.7, 0.6),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_load_noise_attribution")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("summary_rebuild_stream_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_split_window_query_load_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite =
        |turn_count_ms: f64, known_overflow_rows_ms: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                target_load_ms: vec![1.0, 1.0],
                target_load_window_query_ms: vec![
                    turn_count_ms + known_overflow_rows_ms,
                    turn_count_ms + known_overflow_rows_ms,
                ],
                target_load_window_turn_count_query_ms: vec![turn_count_ms, turn_count_ms],
                target_load_window_known_overflow_rows_query_ms: vec![
                    known_overflow_rows_ms,
                    known_overflow_rows_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &[
            make_suite(0.3, 1.0),
            make_suite(0.35, 4.0),
            make_suite(0.4, 8.0),
        ],
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_load_noise_attribution")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("window_known_overflow_rows_query_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_summary_rebuild_checkpoint_commit_noise_attribution() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |rebuild_stream_ms: f64,
                      rebuild_metadata_upsert_ms: f64,
                      rebuild_body_upsert_ms: f64,
                      rebuild_commit_ms: f64| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                target_load_summary_rebuild_ms: vec![1.0, 1.0],
                target_load_summary_rebuild_stream_ms: vec![rebuild_stream_ms, rebuild_stream_ms],
                target_load_summary_rebuild_checkpoint_metadata_upsert_ms: vec![
                    rebuild_metadata_upsert_ms,
                    rebuild_metadata_upsert_ms,
                ],
                target_load_summary_rebuild_checkpoint_body_upsert_ms: vec![
                    rebuild_body_upsert_ms,
                    rebuild_body_upsert_ms,
                ],
                target_load_summary_rebuild_checkpoint_commit_ms: vec![
                    rebuild_commit_ms,
                    rebuild_commit_ms,
                ],
                target_load_summary_rebuild_checkpoint_upsert_ms: vec![
                    rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                    rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                ],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(0.4, 0.3, 0.4, 1.0),
        make_suite(0.5, 0.35, 0.45, 5.0),
        make_suite(0.6, 0.4, 0.5, 9.0),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        3,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_load_noise_attribution")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load"))
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str),
        Some("summary_rebuild_checkpoint_commit_ms")
    );
}

#[test]
fn memory_context_benchmark_report_emits_cold_path_phase_reports() {
    let shape = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 64,
        payload_chars: 128,
    };
    let make_suite = |rebuild_phase: MemoryContextColdPathPhaseSamples,
                      budget_change_phase: MemoryContextColdPathPhaseSamples,
                      metadata_phase: MemoryContextColdPathPhaseSamples,
                      shrink_phase: MemoryContextColdPathPhaseSamples| {
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: rebuild_phase,
            summary_rebuild_budget_change_phase_samples: budget_change_phase,
            summary_metadata_realign_phase_samples: metadata_phase,
            window_shrink_catch_up_phase_samples: shrink_phase,
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        }
    };
    let suite_runs = vec![
        make_suite(
            MemoryContextColdPathPhaseSamples {
                copy_db_ms: vec![1.0, 1.0],
                target_bootstrap_ms: vec![2.0, 2.0],
                target_load_ms: vec![3.0, 3.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                source_warmup_ms: vec![5.0, 5.0],
                target_load_ms: vec![8.0, 8.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                append_turn_ms: vec![1.5, 1.5],
                target_load_ms: vec![2.0, 2.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                source_warmup_ms: vec![1.0, 1.0],
                target_load_ms: vec![2.0, 2.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
        ),
        make_suite(
            MemoryContextColdPathPhaseSamples {
                copy_db_ms: vec![2.0, 2.0],
                target_bootstrap_ms: vec![4.0, 4.0],
                target_load_ms: vec![7.0, 7.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                source_warmup_ms: vec![9.0, 9.0],
                target_load_ms: vec![12.0, 12.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                append_turn_ms: vec![4.5, 4.5],
                target_load_ms: vec![6.0, 6.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
            MemoryContextColdPathPhaseSamples {
                source_warmup_ms: vec![3.0, 3.0],
                target_load_ms: vec![5.0, 5.0],
                ..MemoryContextColdPathPhaseSamples::default()
            },
        ),
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        2,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    assert_eq!(
        report_json
            .get("cold_path_phases")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load_ms"))
            .and_then(|value| value.get("count"))
            .and_then(Value::as_u64),
        Some(4)
    );
    assert_eq!(
        report_json
            .get("cold_path_phases")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load_ms"))
            .and_then(|value| value.get("p95"))
            .and_then(Value::as_f64),
        Some(7.0)
    );
    assert_eq!(
        report_json
            .get("cold_path_phase_stability")
            .and_then(|value| value.get("summary_rebuild"))
            .and_then(|value| value.get("target_load_ms"))
            .and_then(|value| value.get("range"))
            .and_then(Value::as_f64),
        Some(4.0)
    );
    assert_eq!(
        report_json
            .get("cold_path_phase_stability")
            .and_then(|value| value.get("summary_metadata_realign"))
            .and_then(|value| value.get("append_turn_ms"))
            .and_then(|value| value.get("range"))
            .and_then(Value::as_f64),
        Some(3.0)
    );
}

#[test]
fn memory_context_benchmark_report_tracks_budget_change_workload_adjusted_ratios() {
    let shape_small = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 256,
        payload_chars: 1024,
    };
    let shape_large = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 512,
        payload_chars: 1280,
    };
    let shape_larger = MemoryContextShape {
        entry_count: 2,
        turn_entries: 2,
        summary_chars: 768,
        payload_chars: 1536,
    };
    let suite_runs = vec![
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![2.0, 2.0],
            summary_rebuild_budget_change_samples: vec![3.0, 3.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![1.0, 1.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape_small,
            summary_window_cover_shape: shape_small,
            summary_rebuild_shape: shape_small,
            summary_rebuild_budget_change_shape: shape_large,
            summary_metadata_realign_shape: shape_small,
            summary_steady_state_shape: shape_small,
            window_shrink_catch_up_shape: shape_small,
        },
        MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![2.0, 2.0],
            summary_rebuild_budget_change_samples: vec![3.6, 3.6],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![1.0, 1.0],
            window_shrink_catch_up_samples: vec![1.0, 1.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape_small,
            summary_window_cover_shape: shape_small,
            summary_rebuild_shape: shape_small,
            summary_rebuild_budget_change_shape: shape_larger,
            summary_metadata_realign_shape: shape_small,
            summary_steady_state_shape: shape_small,
            window_shrink_catch_up_shape: shape_small,
        },
    ];

    let report = build_memory_context_benchmark_report(
        "target/benchmarks/memory-context-benchmark-report.json",
        &ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from("target/benchmarks/tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        },
        24,
        6,
        12,
        256,
        12,
        2,
        4,
        1,
        &suite_runs,
        2,
        false,
        1.10,
    );
    let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

    let flattened_raw_ratio = report_json
        .get("flattened_sample_ratios")
        .and_then(|value| value.get("summary_rebuild_budget_change_vs_rebuild_ratio_p95"))
        .and_then(Value::as_f64)
        .expect("raw budget-change ratio should be present");
    let flattened_adjusted_ratio = report_json
        .get("flattened_sample_ratios")
        .and_then(|value| {
            value.get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
        })
        .and_then(Value::as_f64)
        .expect("summary-char-adjusted budget-change ratio should be present");
    let aggregated_adjusted_ratio = report_json
        .get("aggregated_ratios")
        .and_then(|value| {
            value.get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
        })
        .and_then(Value::as_f64)
        .expect("aggregated summary-char-adjusted budget-change ratio should be present");

    assert!((flattened_raw_ratio - 1.8).abs() < 0.001);
    assert!((flattened_adjusted_ratio - 0.72).abs() < 0.001);
    assert!((aggregated_adjusted_ratio - 0.675).abs() < 0.001);
}

#[test]
fn memory_context_hot_read_helper_excludes_warmup_reads_from_samples() {
    let shape = MemoryContextShape {
        entry_count: 4,
        turn_entries: 4,
        summary_chars: 0,
        payload_chars: 128,
    };
    let mut call_count = 0_usize;

    let (latencies, rss_deltas_kib, final_shape) =
        measure_hot_prompt_context_reads_with_loader(0, 2, false, || {
            call_count = call_count.saturating_add(1);
            Ok(PromptContextReadObservation {
                latency_ms: call_count as f64,
                rss_delta_kib: Some((call_count * 10) as f64),
                shape,
            })
        })
        .expect("hot-read helper should preserve only measured samples");

    assert_eq!(call_count, 3);
    assert_eq!(latencies, vec![2.0, 3.0]);
    assert_eq!(rss_deltas_kib, vec![20.0, 30.0]);
    assert_eq!(final_shape.entry_count, shape.entry_count);
    assert_eq!(final_shape.turn_entries, shape.turn_entries);
    assert_eq!(final_shape.summary_chars, shape.summary_chars);
    assert_eq!(final_shape.payload_chars, shape.payload_chars);
}

#[test]
fn memory_context_hot_read_helper_rejects_missing_summary_during_warmup() {
    let error = measure_hot_prompt_context_reads_with_loader(1, 2, true, || {
        Ok(PromptContextReadObservation {
            latency_ms: 1.0,
            rss_delta_kib: Some(8.0),
            shape: MemoryContextShape {
                entry_count: 3,
                turn_entries: 3,
                summary_chars: 0,
                payload_chars: 96,
            },
        })
    })
    .expect_err("summary warmup without a summary should fail");

    assert!(error.contains("summary benchmark warmup did not produce a summary entry"));
}

#[test]
fn benchmark_temp_root_uses_unique_suffixes_per_call() {
    let first = benchmark_temp_root("loong-memory-context-benchmark-test", None);
    let second = benchmark_temp_root("loong-memory-context-benchmark-test", None);

    assert_ne!(first, second);
}

#[test]
fn benchmark_temp_root_honors_requested_parent_directory() {
    let requested_parent = Path::new("/tmp/loong-memory-context-benchmark-parent");
    let root = benchmark_temp_root(
        "loong-memory-context-benchmark-test",
        Some(requested_parent),
    );

    assert_eq!(root.parent(), Some(requested_parent));
}

#[test]
fn memory_context_benchmark_temp_root_prefers_explicit_override() {
    let explicit = Path::new("/tmp/loong-memory-context-benchmark-explicit");
    let resolved = resolve_memory_context_benchmark_temp_root(
        "target/benchmarks/memory-context-benchmark-report.json",
        Some(explicit.to_str().expect("utf-8 explicit path")),
    )
    .expect("resolve explicit temp root");

    assert_eq!(resolved.path, explicit);
    assert_eq!(
        resolved.source,
        MemoryContextBenchmarkTempRootSource::Explicit
    );
}

#[test]
fn memory_context_benchmark_temp_root_defaults_to_output_parent_for_target_reports() {
    let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
        "target/benchmarks/memory-context-benchmark-report.json",
        None,
        None,
    )
    .expect("resolve temp root for target benchmark report");

    assert_eq!(resolved.path, Path::new("target/benchmarks/tmp-local"));
    assert_eq!(
        resolved.source,
        MemoryContextBenchmarkTempRootSource::OutputParent
    );
}

#[test]
fn memory_context_benchmark_temp_root_falls_back_to_system_temp_outside_target() {
    let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
        "/tmp/memory-context-benchmark-report.json",
        None,
        None,
    )
    .expect("resolve temp root outside target");

    assert_eq!(resolved.path, std::env::temp_dir());
    assert_eq!(
        resolved.source,
        MemoryContextBenchmarkTempRootSource::SystemTemp
    );
}

#[test]
fn memory_context_benchmark_temp_root_prefers_current_exe_target_dir() {
    let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
        "target/benchmarks/memory-context-benchmark-report.json",
        None,
        Some(Path::new("/repo/target/codex-memory-bench-red/debug/loong")),
    )
    .expect("resolve temp root from current exe");

    assert_eq!(
        resolved.path,
        Path::new("/repo/target/codex-memory-bench-red/tmp-local")
    );
    assert_eq!(
        resolved.source,
        MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir
    );
}
