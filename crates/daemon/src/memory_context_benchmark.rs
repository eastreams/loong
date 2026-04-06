use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH},
};

use loongclaw_app::{
    config::{MemoryMode, MemoryProfile},
    memory::{
        self, MemoryContextEntry, MemoryContextKind, SqliteBootstrapDiagnostics,
        SqliteContextLoadDiagnostics, runtime_config::MemoryRuntimeConfig,
    },
};
use loongclaw_bench::{
    MemoryContextBenchmarkSuiteSamples, MemoryContextColdPathPhaseSamples, MemoryContextShape,
    copy_benchmark_file,
    run_memory_context_benchmark_cli_with_suite_runner as run_bench_memory_context_benchmark_cli,
};
use rusqlite::Connection;

use crate::CliResult;

pub fn run_memory_context_benchmark_cli(
    output_path: &str,
    temp_root: Option<&str>,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    suite_repetitions: usize,
    enforce_gate: bool,
    min_steady_state_speedup_ratio: f64,
) -> CliResult<()> {
    run_bench_memory_context_benchmark_cli(
        output_path,
        temp_root,
        history_turns,
        sliding_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        suite_repetitions,
        enforce_gate,
        min_steady_state_speedup_ratio,
        run_memory_context_benchmark_suite,
    )
}

#[derive(Debug, Clone, Copy)]
enum MemoryContextBootstrapKind {
    Source,
    Target,
}

#[derive(Debug, Clone)]
struct PromptContextReadObservation {
    latency_ms: f64,
    rss_delta_kib: Option<f64>,
    shape: MemoryContextShape,
    load_diagnostics: SqliteContextLoadDiagnostics,
}

fn run_memory_context_benchmark_suite(
    temp_root_override: Option<&Path>,
    history_turns: usize,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
) -> CliResult<MemoryContextBenchmarkSuiteSamples> {
    let temp_root = benchmark_temp_root("loongclaw-memory-context-benchmark", temp_root_override);
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create memory benchmark temp directory: {error}"))?;

    let result = (|| {
        let session_id = "memory-context-benchmark-session";
        let seed_db = temp_root.join("seed-history.sqlite3");
        seed_memory_context_history(
            &seed_db,
            session_id,
            history_turns,
            sliding_window,
            summary_max_chars,
            words_per_turn,
        )?;
        checkpoint_sqlite_database(&seed_db)?;
        release_memory_benchmark_runtime(&seed_db)?;
        let seed_db_bytes = fs::metadata(&seed_db)
            .map_err(|error| format!("failed to read seed database metadata: {error}"))?
            .len();

        let (window_only_samples, window_only_rss_deltas_kib, window_only_shape) =
            sample_window_only_context(
                &temp_root,
                &seed_db,
                session_id,
                sliding_window,
                warmup_iterations,
                hot_iterations,
            )?;
        let (
            summary_window_cover_samples,
            summary_window_cover_rss_deltas_kib,
            summary_window_cover_shape,
        ) = sample_summary_window_cover_context(
            &temp_root,
            session_id,
            sliding_window,
            summary_max_chars,
            words_per_turn,
            warmup_iterations,
            hot_iterations,
        )?;
        let (
            summary_rebuild_samples,
            summary_rebuild_rss_deltas_kib,
            summary_rebuild_shape,
            summary_rebuild_phase_samples,
        ) = sample_summary_rebuild_context(
            &temp_root,
            &seed_db,
            session_id,
            sliding_window,
            summary_max_chars,
            rebuild_iterations,
        )?;
        let (
            summary_rebuild_budget_change_samples,
            summary_rebuild_budget_change_rss_deltas_kib,
            summary_rebuild_budget_change_shape,
            summary_rebuild_budget_change_phase_samples,
        ) = sample_summary_rebuild_budget_change_context(
            &temp_root,
            &seed_db,
            session_id,
            sliding_window,
            summary_max_chars,
            rebuild_iterations,
        )?;
        let (
            summary_metadata_realign_samples,
            summary_metadata_realign_rss_deltas_kib,
            summary_metadata_realign_shape,
            summary_metadata_realign_phase_samples,
        ) = sample_summary_metadata_realign_context(
            &temp_root,
            &seed_db,
            session_id,
            history_turns,
            sliding_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
        )?;
        let (
            summary_steady_state_samples,
            summary_steady_state_rss_deltas_kib,
            summary_steady_state_shape,
        ) = sample_summary_steady_state_context(
            &temp_root,
            &seed_db,
            session_id,
            sliding_window,
            summary_max_chars,
            warmup_iterations,
            hot_iterations,
        )?;
        let (
            window_shrink_catch_up_samples,
            window_shrink_catch_up_rss_deltas_kib,
            window_shrink_catch_up_shape,
            window_shrink_catch_up_phase_samples,
        ) = sample_window_shrink_catch_up_context(
            &temp_root,
            &seed_db,
            session_id,
            sliding_window,
            window_shrink_source_window,
            summary_max_chars,
            rebuild_iterations,
        )?;
        let (
            window_only_append_pre_overflow_samples,
            window_only_append_pre_overflow_rss_deltas_kib,
        ) = sample_window_only_append_context(
            &temp_root,
            "window-only-append-pre-overflow",
            session_id,
            sliding_window.saturating_sub(1),
            sliding_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
        )?;
        let (
            window_only_append_cold_overflow_samples,
            window_only_append_cold_overflow_rss_deltas_kib,
        ) = sample_window_only_append_context(
            &temp_root,
            "window-only-append-cold-overflow",
            session_id,
            sliding_window,
            sliding_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
        )?;
        let (summary_append_pre_overflow_samples, summary_append_pre_overflow_rss_deltas_kib) =
            sample_summary_append_pre_overflow_context(
                &temp_root,
                session_id,
                sliding_window,
                summary_max_chars,
                words_per_turn,
                rebuild_iterations,
            )?;
        let (summary_append_cold_overflow_samples, summary_append_cold_overflow_rss_deltas_kib) =
            sample_summary_append_cold_overflow_context(
                &temp_root,
                session_id,
                sliding_window,
                summary_max_chars,
                words_per_turn,
                rebuild_iterations,
            )?;
        let (summary_append_saturated_samples, summary_append_saturated_rss_deltas_kib) =
            sample_summary_append_saturated_context(
                &temp_root,
                &seed_db,
                session_id,
                history_turns,
                sliding_window,
                summary_max_chars,
                words_per_turn,
                warmup_iterations,
                hot_iterations,
            )?;

        Ok(MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes,
            window_only_samples,
            summary_window_cover_samples,
            summary_rebuild_samples,
            summary_rebuild_budget_change_samples,
            summary_metadata_realign_samples,
            summary_steady_state_samples,
            window_shrink_catch_up_samples,
            window_only_append_pre_overflow_samples,
            window_only_append_cold_overflow_samples,
            summary_append_pre_overflow_samples,
            summary_append_cold_overflow_samples,
            summary_append_saturated_samples,
            window_only_rss_deltas_kib,
            summary_window_cover_rss_deltas_kib,
            summary_rebuild_rss_deltas_kib,
            summary_rebuild_budget_change_rss_deltas_kib,
            summary_metadata_realign_rss_deltas_kib,
            summary_steady_state_rss_deltas_kib,
            window_shrink_catch_up_rss_deltas_kib,
            window_only_append_pre_overflow_rss_deltas_kib,
            window_only_append_cold_overflow_rss_deltas_kib,
            summary_append_pre_overflow_rss_deltas_kib,
            summary_append_cold_overflow_rss_deltas_kib,
            summary_append_saturated_rss_deltas_kib,
            summary_rebuild_phase_samples,
            summary_rebuild_budget_change_phase_samples,
            summary_metadata_realign_phase_samples,
            window_shrink_catch_up_phase_samples,
            window_only_shape,
            summary_window_cover_shape,
            summary_rebuild_shape,
            summary_rebuild_budget_change_shape,
            summary_metadata_realign_shape,
            summary_steady_state_shape,
            window_shrink_catch_up_shape,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    result
}

fn sample_window_only_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    sliding_window: usize,
    warmup_iterations: usize,
    hot_iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    let db_path = temp_root.join("window-only.sqlite3");
    let result = (|| {
        copy_benchmark_file(seed_db, &db_path).map_err(|error| {
            format!("failed to prepare window-only benchmark database: {error}")
        })?;
        let config = memory_window_only_config(db_path.clone(), sliding_window, 256);
        memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .map_err(|error| format!("window-only benchmark bootstrap failed: {error}"))?;
        measure_hot_prompt_context_reads(
            session_id,
            &config,
            warmup_iterations,
            hot_iterations,
            false,
        )
    })();
    finalize_memory_benchmark_runtime(&db_path, result)
}

fn sample_summary_window_cover_context(
    temp_root: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    warmup_iterations: usize,
    hot_iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    let db_path = temp_root.join("summary-window-cover.sqlite3");
    let result = (|| {
        seed_memory_context_history(
            &db_path,
            session_id,
            sliding_window,
            sliding_window,
            summary_max_chars,
            words_per_turn,
        )?;
        checkpoint_sqlite_database(&db_path)?;
        // Keep this seeded-db path aligned with the other summary scenarios so
        // the read samples do not inherit a live writer connection.
        release_memory_benchmark_runtime(&db_path)?;
        let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
        memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .map_err(|error| format!("summary window-cover benchmark bootstrap failed: {error}"))?;
        measure_hot_prompt_context_reads(
            session_id,
            &config,
            warmup_iterations,
            hot_iterations,
            false,
        )
    })();
    finalize_memory_benchmark_runtime(&db_path, result)
}

fn sample_summary_rebuild_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    rebuild_iterations: usize,
) -> CliResult<(
    Vec<f64>,
    Vec<f64>,
    MemoryContextShape,
    MemoryContextColdPathPhaseSamples,
)> {
    let mut latencies = Vec::with_capacity(rebuild_iterations);
    let mut rss_deltas_kib = Vec::with_capacity(rebuild_iterations);
    let mut phase_samples = MemoryContextColdPathPhaseSamples::default();
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };

    for iteration in 0..rebuild_iterations {
        let db_path = temp_root.join(format!("summary-rebuild-{iteration}.sqlite3"));
        let iteration_result = (|| {
            measure_benchmark_phase(&mut phase_samples.copy_db_ms, || {
                copy_benchmark_file(seed_db, &db_path).map_err(|error| {
                    format!("failed to prepare summary rebuild benchmark database: {error}")
                })?;
                Ok(())
            })?;
            let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Target,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
                        .map_err(|error| {
                            format!("summary rebuild benchmark bootstrap failed: {error}")
                        })
                },
            )?;
            let (samples, rss_samples_kib, shape, load_diagnostics) =
                measure_prompt_context_reads(session_id, &config, 1, true)?;
            phase_samples.target_load_ms.extend(samples.iter().copied());
            for diagnostics in &load_diagnostics {
                record_memory_context_load_diagnostics(&mut phase_samples, diagnostics);
            }
            Ok((samples, rss_samples_kib, shape))
        })();
        let (samples, rss_samples_kib, shape) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.extend(samples);
        rss_deltas_kib.extend(rss_samples_kib);
        final_shape = shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape, phase_samples))
}

fn sample_summary_rebuild_budget_change_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    rebuild_iterations: usize,
) -> CliResult<(
    Vec<f64>,
    Vec<f64>,
    MemoryContextShape,
    MemoryContextColdPathPhaseSamples,
)> {
    let mut latencies = Vec::with_capacity(rebuild_iterations);
    let mut rss_deltas_kib = Vec::with_capacity(rebuild_iterations);
    let mut phase_samples = MemoryContextColdPathPhaseSamples::default();
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };
    let source_summary_max_chars = summary_max_chars.max(256);
    let target_summary_max_chars =
        source_summary_max_chars.saturating_add(source_summary_max_chars.max(256));
    if target_summary_max_chars == source_summary_max_chars {
        return Err(
            "summary rebuild budget-change benchmark could not derive a distinct target budget"
                .to_owned(),
        );
    }

    for iteration in 0..rebuild_iterations {
        let db_path = temp_root.join(format!("summary-rebuild-budget-change-{iteration}.sqlite3"));
        let iteration_result = (|| {
            measure_benchmark_phase(&mut phase_samples.copy_db_ms, || {
                copy_benchmark_file(seed_db, &db_path).map_err(|error| {
                    format!(
                        "failed to prepare summary rebuild budget-change benchmark database: {error}"
                    )
                })?;
                Ok(())
            })?;

            let source_config =
                memory_summary_config(db_path.clone(), sliding_window, source_summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Source,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &source_config,
                    )
                    .map_err(|error| {
                        format!("summary rebuild budget-change source bootstrap failed: {error}")
                    })
                },
            )?;
            let source_entries =
                measure_benchmark_phase(&mut phase_samples.source_warmup_ms, || {
                    memory::load_prompt_context(session_id, &source_config).map_err(|error| {
                        format!("summary rebuild budget-change source warmup failed: {error}")
                    })
                })?;
            let source_shape = memory_context_shape(&source_entries);
            if source_shape.summary_chars == 0 {
                return Err(
                    "summary rebuild budget-change source warmup did not materialize a summary entry"
                        .to_owned(),
                );
            }

            let target_config =
                memory_summary_config(db_path.clone(), sliding_window, target_summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Target,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &target_config,
                    )
                    .map_err(|error| {
                        format!("summary rebuild budget-change target bootstrap failed: {error}")
                    })
                },
            )?;
            let (samples, rss_samples_kib, shape, load_diagnostics) =
                measure_prompt_context_reads(session_id, &target_config, 1, true)?;
            phase_samples.target_load_ms.extend(samples.iter().copied());
            for diagnostics in &load_diagnostics {
                record_memory_context_load_diagnostics(&mut phase_samples, diagnostics);
            }
            Ok((samples, rss_samples_kib, shape))
        })();
        let (samples, rss_samples_kib, shape) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.extend(samples);
        rss_deltas_kib.extend(rss_samples_kib);
        final_shape = shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape, phase_samples))
}

fn sample_summary_metadata_realign_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    iterations: usize,
) -> CliResult<(
    Vec<f64>,
    Vec<f64>,
    MemoryContextShape,
    MemoryContextColdPathPhaseSamples,
)> {
    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);
    let mut phase_samples = MemoryContextColdPathPhaseSamples::default();
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };

    if sliding_window <= 1 {
        return Ok((latencies, rss_deltas_kib, final_shape, phase_samples));
    }

    let source_window = sliding_window - 1;

    for iteration in 0..iterations {
        let db_path = temp_root.join(format!("summary-metadata-realign-{iteration}.sqlite3"));
        let iteration_result = (|| {
            measure_benchmark_phase(&mut phase_samples.copy_db_ms, || {
                copy_benchmark_file(seed_db, &db_path).map_err(|error| {
                    format!(
                        "failed to prepare summary metadata-realign benchmark database: {error}"
                    )
                })?;
                Ok(())
            })?;

            let source_config =
                memory_summary_config(db_path.clone(), source_window, summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Source,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &source_config,
                    )
                    .map_err(|error| {
                        format!("summary metadata-realign source bootstrap failed: {error}")
                    })
                },
            )?;
            let source_entries =
                measure_benchmark_phase(&mut phase_samples.source_warmup_ms, || {
                    memory::load_prompt_context(session_id, &source_config).map_err(|error| {
                        format!("summary metadata-realign source warmup failed: {error}")
                    })
                })?;
            let source_shape = memory_context_shape(&source_entries);
            if source_shape.summary_chars == 0 {
                return Err(
                    "summary metadata-realign source warmup did not materialize a summary entry"
                        .to_owned(),
                );
            }

            let window_only_config =
                memory_window_only_config(db_path.clone(), sliding_window, summary_max_chars);
            measure_benchmark_phase(&mut phase_samples.append_turn_ms, || {
                append_benchmark_turn(
                    session_id,
                    &window_only_config,
                    history_turns,
                    words_per_turn,
                )
            })?;

            let target_config =
                memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Target,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &target_config,
                    )
                    .map_err(|error| {
                        format!("summary metadata-realign target bootstrap failed: {error}")
                    })
                },
            )?;
            let (samples, rss_samples_kib, shape, load_diagnostics) =
                measure_prompt_context_reads(session_id, &target_config, 1, true)?;
            phase_samples.target_load_ms.extend(samples.iter().copied());
            for diagnostics in &load_diagnostics {
                record_memory_context_load_diagnostics(&mut phase_samples, diagnostics);
            }
            Ok((samples, rss_samples_kib, shape))
        })();
        let (samples, rss_samples_kib, shape) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.extend(samples);
        rss_deltas_kib.extend(rss_samples_kib);
        final_shape = shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape, phase_samples))
}

fn sample_summary_steady_state_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    warmup_iterations: usize,
    hot_iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    let db_path = temp_root.join("summary-steady-state.sqlite3");
    let result = (|| {
        copy_benchmark_file(seed_db, &db_path).map_err(|error| {
            format!("failed to prepare summary steady-state benchmark database: {error}")
        })?;
        let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
        memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .map_err(|error| format!("summary steady-state benchmark bootstrap failed: {error}"))?;
        measure_hot_prompt_context_reads(
            session_id,
            &config,
            warmup_iterations,
            hot_iterations,
            true,
        )
    })();
    finalize_memory_benchmark_runtime(&db_path, result)
}

fn sample_window_shrink_catch_up_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    shrink_iterations: usize,
) -> CliResult<(
    Vec<f64>,
    Vec<f64>,
    MemoryContextShape,
    MemoryContextColdPathPhaseSamples,
)> {
    let mut latencies = Vec::with_capacity(shrink_iterations);
    let mut rss_deltas_kib = Vec::with_capacity(shrink_iterations);
    let mut phase_samples = MemoryContextColdPathPhaseSamples::default();
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };

    for iteration in 0..shrink_iterations {
        let db_path = temp_root.join(format!("window-shrink-catch-up-{iteration}.sqlite3"));
        let iteration_result = (|| {
            measure_benchmark_phase(&mut phase_samples.copy_db_ms, || {
                copy_benchmark_file(seed_db, &db_path).map_err(|error| {
                    format!("failed to prepare shrink catch-up benchmark database: {error}")
                })?;
                Ok(())
            })?;

            let source_config = memory_summary_config(
                db_path.clone(),
                window_shrink_source_window,
                summary_max_chars,
            );
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Source,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &source_config,
                    )
                    .map_err(|error| format!("shrink catch-up source bootstrap failed: {error}"))
                },
            )?;
            let source_entries =
                measure_benchmark_phase(&mut phase_samples.source_warmup_ms, || {
                    memory::load_prompt_context(session_id, &source_config)
                        .map_err(|error| format!("shrink catch-up source warmup failed: {error}"))
                })?;
            let source_shape = memory_context_shape(&source_entries);
            if source_shape.summary_chars == 0 {
                return Err(
                    "shrink catch-up benchmark source warmup did not materialize a summary entry"
                        .to_owned(),
                );
            }

            let target_config =
                memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
            let _ = measure_memory_context_bootstrap_phase(
                &mut phase_samples,
                MemoryContextBootstrapKind::Target,
                || {
                    memory::ensure_memory_db_ready_with_diagnostics(
                        Some(db_path.clone()),
                        &target_config,
                    )
                    .map_err(|error| format!("shrink catch-up target bootstrap failed: {error}"))
                },
            )?;
            let (samples, rss_samples_kib, shape, load_diagnostics) =
                measure_prompt_context_reads(session_id, &target_config, 1, true)?;
            phase_samples.target_load_ms.extend(samples.iter().copied());
            for diagnostics in &load_diagnostics {
                record_memory_context_load_diagnostics(&mut phase_samples, diagnostics);
            }
            Ok((samples, rss_samples_kib, shape))
        })();
        let (samples, rss_samples_kib, shape) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.extend(samples);
        rss_deltas_kib.extend(rss_samples_kib);
        final_shape = shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape, phase_samples))
}

fn sample_window_only_append_context(
    temp_root: &Path,
    scenario_name: &str,
    session_id: &str,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let baseline_db = temp_root.join(format!("{scenario_name}-baseline.sqlite3"));
    seed_memory_context_history(
        &baseline_db,
        session_id,
        history_turns,
        sliding_window,
        summary_max_chars,
        words_per_turn,
    )?;
    checkpoint_sqlite_database(&baseline_db)?;
    release_memory_benchmark_runtime(&baseline_db)?;

    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);

    for iteration in 0..iterations {
        let db_path = temp_root.join(format!("{scenario_name}-{iteration}.sqlite3"));
        let iteration_result = (|| {
            copy_benchmark_file(&baseline_db, &db_path).map_err(|error| {
                format!("failed to prepare {scenario_name} benchmark database: {error}")
            })?;
            let config =
                memory_window_only_config(db_path.clone(), sliding_window, summary_max_chars);
            memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
                .map_err(|error| format!("{scenario_name} benchmark bootstrap failed: {error}"))?;

            let baseline_rss_kib = sample_process_rss_kib();
            let started_at = StdInstant::now();
            append_benchmark_turn(session_id, &config, history_turns, words_per_turn)?;
            let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let rss_delta_kib =
                compute_rss_step_delta_kib(baseline_rss_kib, sample_process_rss_kib());
            Ok((latency_ms, rss_delta_kib))
        })();
        let (latency_ms, rss_delta_kib) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.push(latency_ms);
        if let Some(delta_kib) = rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
    }

    Ok((latencies, rss_deltas_kib))
}

fn sample_summary_append_saturated_context(
    temp_root: &Path,
    seed_db: &Path,
    session_id: &str,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    warmup_iterations: usize,
    hot_iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let db_path = temp_root.join("summary-append-saturated.sqlite3");
    let result = (|| {
        copy_benchmark_file(seed_db, &db_path).map_err(|error| {
            format!("failed to prepare summary append saturated benchmark database: {error}")
        })?;
        let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
        memory::ensure_memory_db_ready(Some(db_path.clone()), &config).map_err(|error| {
            format!("summary append saturated benchmark bootstrap failed: {error}")
        })?;

        let entries = memory::load_prompt_context(session_id, &config)
            .map_err(|error| format!("summary append saturated warmup failed: {error}"))?;
        let shape = memory_context_shape(&entries);
        if shape.summary_chars == 0 {
            return Err(
                "summary append saturated warmup did not materialize a summary entry".to_owned(),
            );
        }

        let mut next_turn_index = history_turns;
        for _ in 0..warmup_iterations.max(1) {
            append_benchmark_turn(session_id, &config, next_turn_index, words_per_turn)?;
            next_turn_index = next_turn_index.saturating_add(1);
        }

        measure_summary_append_latencies(
            session_id,
            &config,
            next_turn_index,
            words_per_turn,
            hot_iterations,
        )
    })();
    finalize_memory_benchmark_runtime(&db_path, result)
}

fn sample_summary_append_cold_overflow_context(
    temp_root: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let baseline_db = temp_root.join("summary-append-cold-overflow-baseline.sqlite3");
    seed_memory_context_history(
        &baseline_db,
        session_id,
        sliding_window,
        sliding_window,
        summary_max_chars,
        words_per_turn,
    )?;
    checkpoint_sqlite_database(&baseline_db)?;
    release_memory_benchmark_runtime(&baseline_db)?;

    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);

    for iteration in 0..iterations {
        let db_path = temp_root.join(format!("summary-append-cold-overflow-{iteration}.sqlite3"));
        let iteration_result = (|| {
            copy_benchmark_file(&baseline_db, &db_path).map_err(|error| {
                format!(
                    "failed to prepare summary append cold overflow benchmark database: {error}"
                )
            })?;
            let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
            memory::ensure_memory_db_ready(Some(db_path.clone()), &config).map_err(|error| {
                format!("summary append cold overflow benchmark bootstrap failed: {error}")
            })?;

            let baseline_rss_kib = sample_process_rss_kib();
            let started_at = StdInstant::now();
            append_benchmark_turn(session_id, &config, sliding_window, words_per_turn)?;
            let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let rss_delta_kib =
                compute_rss_step_delta_kib(baseline_rss_kib, sample_process_rss_kib());
            Ok((latency_ms, rss_delta_kib))
        })();
        let (latency_ms, rss_delta_kib) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.push(latency_ms);
        if let Some(delta_kib) = rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
    }

    Ok((latencies, rss_deltas_kib))
}

fn sample_summary_append_pre_overflow_context(
    temp_root: &Path,
    session_id: &str,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let baseline_db = temp_root.join("summary-append-pre-overflow-baseline.sqlite3");
    seed_memory_context_history(
        &baseline_db,
        session_id,
        sliding_window.saturating_sub(1),
        sliding_window,
        summary_max_chars,
        words_per_turn,
    )?;
    checkpoint_sqlite_database(&baseline_db)?;
    release_memory_benchmark_runtime(&baseline_db)?;

    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);

    for iteration in 0..iterations {
        let db_path = temp_root.join(format!("summary-append-pre-overflow-{iteration}.sqlite3"));
        let iteration_result = (|| {
            copy_benchmark_file(&baseline_db, &db_path).map_err(|error| {
                format!("failed to prepare summary append pre-overflow benchmark database: {error}")
            })?;
            let config = memory_summary_config(db_path.clone(), sliding_window, summary_max_chars);
            memory::ensure_memory_db_ready(Some(db_path.clone()), &config).map_err(|error| {
                format!("summary append pre-overflow benchmark bootstrap failed: {error}")
            })?;

            let baseline_rss_kib = sample_process_rss_kib();
            let started_at = StdInstant::now();
            append_benchmark_turn(
                session_id,
                &config,
                sliding_window.saturating_sub(1),
                words_per_turn,
            )?;
            let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let rss_delta_kib =
                compute_rss_step_delta_kib(baseline_rss_kib, sample_process_rss_kib());
            Ok((latency_ms, rss_delta_kib))
        })();
        let (latency_ms, rss_delta_kib) =
            finalize_memory_benchmark_runtime(&db_path, iteration_result)?;
        latencies.push(latency_ms);
        if let Some(delta_kib) = rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
    }

    Ok((latencies, rss_deltas_kib))
}

fn measure_prompt_context_reads(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    iterations: usize,
    expect_summary: bool,
) -> CliResult<(
    Vec<f64>,
    Vec<f64>,
    MemoryContextShape,
    Vec<SqliteContextLoadDiagnostics>,
)> {
    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);
    let mut load_diagnostics = Vec::with_capacity(iterations);
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };
    for _ in 0..iterations {
        let observation = load_prompt_context_observation(session_id, config)?;
        latencies.push(observation.latency_ms);
        if let Some(delta_kib) = observation.rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
        validate_prompt_context_shape(observation.shape, expect_summary, "sample")?;
        final_shape = observation.shape;
        load_diagnostics.push(observation.load_diagnostics);
    }

    Ok((latencies, rss_deltas_kib, final_shape, load_diagnostics))
}

fn measure_benchmark_phase<T>(
    phase_samples_ms: &mut Vec<f64>,
    operation: impl FnOnce() -> CliResult<T>,
) -> CliResult<T> {
    let started_at = StdInstant::now();
    let result = operation();
    if result.is_ok() {
        phase_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
    }
    result
}

fn measure_memory_context_bootstrap_phase(
    phase_samples: &mut MemoryContextColdPathPhaseSamples,
    bootstrap_kind: MemoryContextBootstrapKind,
    operation: impl FnOnce() -> CliResult<(PathBuf, SqliteBootstrapDiagnostics)>,
) -> CliResult<PathBuf> {
    let (path, diagnostics) = operation()?;
    record_memory_context_bootstrap_diagnostics(phase_samples, bootstrap_kind, &diagnostics);
    Ok(path)
}

fn record_memory_context_bootstrap_diagnostics(
    phase_samples: &mut MemoryContextColdPathPhaseSamples,
    bootstrap_kind: MemoryContextBootstrapKind,
    diagnostics: &SqliteBootstrapDiagnostics,
) {
    match bootstrap_kind {
        MemoryContextBootstrapKind::Source => {
            phase_samples.source_bootstrap_ms.push(diagnostics.total_ms);
            phase_samples
                .source_bootstrap_normalize_path_ms
                .push(diagnostics.normalize_path_ms);
            phase_samples
                .source_bootstrap_registry_lock_ms
                .push(diagnostics.registry_lock_ms);
            phase_samples
                .source_bootstrap_registry_lookup_ms
                .push(diagnostics.registry_lookup_ms);
            phase_samples
                .source_bootstrap_runtime_create_ms
                .push(diagnostics.runtime_create_ms);
            phase_samples
                .source_bootstrap_parent_dir_create_ms
                .push(diagnostics.parent_dir_create_ms);
            phase_samples
                .source_bootstrap_connection_open_ms
                .push(diagnostics.connection_open_ms);
            phase_samples
                .source_bootstrap_configure_connection_ms
                .push(diagnostics.configure_connection_ms);
            phase_samples
                .source_bootstrap_schema_init_ms
                .push(diagnostics.schema_init_ms);
            phase_samples
                .source_bootstrap_schema_upgrade_ms
                .push(diagnostics.schema_upgrade_ms);
            phase_samples
                .source_bootstrap_registry_insert_ms
                .push(diagnostics.registry_insert_ms);
        }
        MemoryContextBootstrapKind::Target => {
            phase_samples.target_bootstrap_ms.push(diagnostics.total_ms);
            phase_samples
                .target_bootstrap_normalize_path_ms
                .push(diagnostics.normalize_path_ms);
            phase_samples
                .target_bootstrap_registry_lock_ms
                .push(diagnostics.registry_lock_ms);
            phase_samples
                .target_bootstrap_registry_lookup_ms
                .push(diagnostics.registry_lookup_ms);
            phase_samples
                .target_bootstrap_runtime_create_ms
                .push(diagnostics.runtime_create_ms);
            phase_samples
                .target_bootstrap_parent_dir_create_ms
                .push(diagnostics.parent_dir_create_ms);
            phase_samples
                .target_bootstrap_connection_open_ms
                .push(diagnostics.connection_open_ms);
            phase_samples
                .target_bootstrap_configure_connection_ms
                .push(diagnostics.configure_connection_ms);
            phase_samples
                .target_bootstrap_schema_init_ms
                .push(diagnostics.schema_init_ms);
            phase_samples
                .target_bootstrap_schema_upgrade_ms
                .push(diagnostics.schema_upgrade_ms);
            phase_samples
                .target_bootstrap_registry_insert_ms
                .push(diagnostics.registry_insert_ms);
        }
    }
}

fn record_memory_context_load_diagnostics(
    phase_samples: &mut MemoryContextColdPathPhaseSamples,
    diagnostics: &SqliteContextLoadDiagnostics,
) {
    phase_samples
        .target_load_window_query_ms
        .push(diagnostics.window_query_ms);
    phase_samples
        .target_load_window_turn_count_query_ms
        .push(diagnostics.window_turn_count_query_ms);
    phase_samples
        .target_load_window_exact_rows_query_ms
        .push(diagnostics.window_exact_rows_query_ms);
    phase_samples
        .target_load_window_known_overflow_rows_query_ms
        .push(diagnostics.window_known_overflow_rows_query_ms);
    phase_samples
        .target_load_window_fallback_rows_query_ms
        .push(diagnostics.window_fallback_rows_query_ms);
    phase_samples
        .target_load_summary_checkpoint_meta_query_ms
        .push(diagnostics.summary_checkpoint_meta_query_ms);
    phase_samples
        .target_load_summary_checkpoint_body_load_ms
        .push(diagnostics.summary_checkpoint_body_load_ms);
    phase_samples
        .target_load_summary_checkpoint_metadata_update_ms
        .push(diagnostics.summary_checkpoint_metadata_update_ms);
    phase_samples
        .target_load_summary_checkpoint_metadata_update_returning_body_ms
        .push(diagnostics.summary_checkpoint_metadata_update_returning_body_ms);
    phase_samples
        .target_load_summary_rebuild_ms
        .push(diagnostics.summary_rebuild_ms);
    phase_samples
        .target_load_summary_rebuild_stream_ms
        .push(diagnostics.summary_rebuild_stream_ms);
    phase_samples
        .target_load_summary_rebuild_checkpoint_upsert_ms
        .push(diagnostics.summary_rebuild_checkpoint_upsert_ms);
    phase_samples
        .target_load_summary_rebuild_checkpoint_metadata_upsert_ms
        .push(diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms);
    phase_samples
        .target_load_summary_rebuild_checkpoint_body_upsert_ms
        .push(diagnostics.summary_rebuild_checkpoint_body_upsert_ms);
    phase_samples
        .target_load_summary_rebuild_checkpoint_commit_ms
        .push(diagnostics.summary_rebuild_checkpoint_commit_ms);
    phase_samples
        .target_load_summary_catch_up_ms
        .push(diagnostics.summary_catch_up_ms);
}

fn measure_hot_prompt_context_reads(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    warmup_iterations: usize,
    hot_iterations: usize,
    expect_summary: bool,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    measure_hot_prompt_context_reads_with_loader(
        warmup_iterations,
        hot_iterations,
        expect_summary,
        || load_prompt_context_observation(session_id, config),
    )
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

fn load_prompt_context_observation(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> CliResult<PromptContextReadObservation> {
    let baseline_rss_kib = sample_process_rss_kib();
    let start = StdInstant::now();
    let (entries, load_diagnostics) =
        memory::load_prompt_context_with_diagnostics(session_id, config)
            .map_err(|error| format!("memory context benchmark read failed: {error}"))?;
    Ok(PromptContextReadObservation {
        latency_ms: start.elapsed().as_secs_f64() * 1000.0,
        rss_delta_kib: compute_rss_step_delta_kib(baseline_rss_kib, sample_process_rss_kib()),
        shape: memory_context_shape(&entries),
        load_diagnostics,
    })
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

fn measure_summary_append_latencies(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    start_turn_index: usize,
    words_per_turn: usize,
    iterations: usize,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let mut latencies = Vec::with_capacity(iterations);
    let mut rss_deltas_kib = Vec::with_capacity(iterations);

    for iteration in 0..iterations {
        let turn_index = start_turn_index.saturating_add(iteration);
        let baseline_rss_kib = sample_process_rss_kib();
        let started_at = StdInstant::now();
        append_benchmark_turn(session_id, config, turn_index, words_per_turn)?;
        latencies.push(started_at.elapsed().as_secs_f64() * 1000.0);
        if let Some(delta_kib) =
            compute_rss_step_delta_kib(baseline_rss_kib, sample_process_rss_kib())
        {
            rss_deltas_kib.push(delta_kib);
        }
    }

    Ok((latencies, rss_deltas_kib))
}

fn append_benchmark_turn(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    turn_index: usize,
    words_per_turn: usize,
) -> CliResult<()> {
    let role = if turn_index.is_multiple_of(2) {
        "user"
    } else {
        "assistant"
    };
    let content = build_memory_context_turn_content(turn_index, words_per_turn);
    memory::append_turn_direct(session_id, role, &content, config).map_err(|error| {
        format!("failed to append memory context benchmark turn {turn_index}: {error}")
    })?;
    Ok(())
}

fn sample_process_rss_kib() -> Option<f64> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", pid.as_str()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    parse_ps_rss_kib_output(&String::from_utf8_lossy(&output.stdout))
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

fn memory_context_shape(entries: &[MemoryContextEntry]) -> MemoryContextShape {
    let mut turn_entries = 0usize;
    let mut summary_chars = 0usize;
    let mut payload_chars = 0usize;
    for entry in entries {
        payload_chars = payload_chars
            .saturating_add(entry.role.len())
            .saturating_add(entry.content.len());
        match entry.kind {
            MemoryContextKind::Turn => {
                turn_entries = turn_entries.saturating_add(1);
            }
            MemoryContextKind::Summary => {
                summary_chars = summary_chars.saturating_add(entry.content.len());
            }
            MemoryContextKind::Profile => {}
            MemoryContextKind::RetrievedMemory => {}
        }
    }

    MemoryContextShape {
        entry_count: entries.len(),
        turn_entries,
        summary_chars,
        payload_chars,
    }
}

fn seed_memory_context_history(
    db_path: &Path,
    session_id: &str,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
) -> CliResult<()> {
    let _ = fs::remove_file(db_path);
    let config =
        memory_window_only_config(db_path.to_path_buf(), sliding_window, summary_max_chars);

    for turn_index in 0..history_turns {
        let role = if turn_index % 2 == 0 {
            "user"
        } else {
            "assistant"
        };
        let content = build_memory_context_turn_content(turn_index, words_per_turn);
        memory::append_turn_direct(session_id, role, &content, &config).map_err(|error| {
            format!("failed to seed memory context benchmark history at turn {turn_index}: {error}")
        })?;
    }

    Ok(())
}

fn checkpoint_sqlite_database(db_path: &Path) -> CliResult<()> {
    let connection = Connection::open(db_path).map_err(|error| {
        format!("failed to open seeded benchmark database for checkpoint: {error}")
    })?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|error| format!("failed to configure checkpoint busy timeout: {error}"))?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| format!("failed to checkpoint seeded benchmark database: {error}"))?;
    Ok(())
}

fn release_memory_benchmark_runtime(db_path: &Path) -> CliResult<()> {
    memory::drop_cached_sqlite_runtime(db_path)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "failed to release cached benchmark sqlite runtime {}: {error}",
                db_path.display()
            )
        })
}

fn finalize_memory_benchmark_runtime<T>(db_path: &Path, result: CliResult<T>) -> CliResult<T> {
    let cleanup_result = release_memory_benchmark_runtime(db_path);
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn memory_window_only_config(
    sqlite_path: PathBuf,
    sliding_window: usize,
    summary_max_chars: usize,
) -> MemoryRuntimeConfig {
    MemoryRuntimeConfig {
        profile: MemoryProfile::WindowOnly,
        mode: MemoryMode::WindowOnly,
        sqlite_path: Some(sqlite_path),
        sliding_window,
        summary_max_chars,
        ..MemoryRuntimeConfig::default()
    }
}

fn memory_summary_config(
    sqlite_path: PathBuf,
    sliding_window: usize,
    summary_max_chars: usize,
) -> MemoryRuntimeConfig {
    MemoryRuntimeConfig {
        profile: MemoryProfile::WindowPlusSummary,
        mode: MemoryMode::WindowPlusSummary,
        sqlite_path: Some(sqlite_path),
        sliding_window,
        summary_max_chars,
        ..MemoryRuntimeConfig::default()
    }
}

fn build_memory_context_turn_content(turn_index: usize, words_per_turn: usize) -> String {
    let mut content = String::new();
    for word_index in 0..words_per_turn {
        if word_index > 0 {
            if word_index % 5 == 0 {
                content.push('\n');
            } else if word_index % 3 == 0 {
                content.push('\t');
            } else {
                content.push(' ');
            }
        }
        content.push_str("turn");
        content.push_str(&turn_index.to_string());
        content.push('_');
        content.push_str(&word_index.to_string());
    }
    content
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

fn next_benchmark_temp_suffix() -> u64 {
    static BENCHMARK_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    BENCHMARK_TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
}

fn benchmark_temp_root(prefix: &str, parent: Option<&Path>) -> PathBuf {
    let parent = match parent {
        Some(parent) => parent.to_path_buf(),
        None => std::env::temp_dir(),
    };
    parent.join(format!(
        "{prefix}-{}-{}-{}",
        current_epoch_seconds(),
        std::process::id(),
        next_benchmark_temp_suffix()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_context_shape_excludes_retrieved_memory_from_summary_counts() {
        let retrieved_entry = MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content: "durable recall".to_owned(),
            provenance: Vec::new(),
        };
        let summary_entry = MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: "summary block".to_owned(),
            provenance: Vec::new(),
        };
        let turn_entry = MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: "user".to_owned(),
            content: "hello".to_owned(),
            provenance: Vec::new(),
        };
        let entries = vec![retrieved_entry, summary_entry, turn_entry];
        let retrieved_payload_chars = "system".len() + "durable recall".len();
        let summary_payload_chars = "system".len() + "summary block".len();
        let turn_payload_chars = "user".len() + "hello".len();
        let expected_payload_chars =
            retrieved_payload_chars + summary_payload_chars + turn_payload_chars;

        let shape = memory_context_shape(entries.as_slice());

        assert_eq!(shape.entry_count, 3);
        assert_eq!(shape.turn_entries, 1);
        assert_eq!(shape.summary_chars, "summary block".len());
        assert_eq!(shape.payload_chars, expected_payload_chars);
    }
}
