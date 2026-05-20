use super::{NumericStats, current_epoch_seconds, write_json_file};
use kernel::{BridgeSupportMatrix, ChannelConfig, ConnectorCommand, ProviderConfig};
use loong_spec::{
    BridgeRuntimePolicy, CliResult, ConnectorCircuitBreakerPolicy, execute_wasm_component_bridge,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::Instant as StdInstant,
};

const DEFAULT_WASM_CACHE_MIN_SPEEDUP_RATIO: f64 = 1.5;
const BENCHMARK_COPY_STRATEGY_ENV: &str = "LOONG_BENCHMARK_COPY_STRATEGY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkCopyStrategy {
    StableFsCopy,
    #[cfg(target_os = "macos")]
    MacosCloneCp,
}

#[derive(Debug, Clone, Serialize)]
struct WasmCacheBenchmarkReport {
    generated_at_epoch_s: u64,
    profile: String,
    input_wasm_path: String,
    effective_wasm_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_note: Option<String>,
    output_path: String,
    cold_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    cold_latency_ms: NumericStats,
    hot_latency_ms: NumericStats,
    cold_cache_hits: usize,
    cold_cache_misses: usize,
    hot_cache_hits: usize,
    hot_cache_misses: usize,
    speedup_ratio_p95: Option<f64>,
    gate: WasmCacheBenchmarkGateSummary,
}

#[derive(Debug, Clone, Serialize)]
struct WasmCacheBenchmarkGateSummary {
    enforced: bool,
    passed: bool,
    min_speedup_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_speedup_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct WasmBridgeSample {
    latency_ms: f64,
    cache_hit: bool,
}

#[allow(clippy::print_stdout)] // CLI benchmark report output
pub fn run_wasm_cache_benchmark_cli(
    wasm_path: &str,
    output_path: &str,
    cold_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    enforce_gate: bool,
    min_speedup_ratio: f64,
) -> CliResult<()> {
    if cold_iterations == 0 {
        return Err("wasm cache benchmark requires cold_iterations > 0".to_owned());
    }
    if hot_iterations == 0 {
        return Err("wasm cache benchmark requires hot_iterations > 0".to_owned());
    }

    let normalized_min_speedup_ratio = if min_speedup_ratio.is_finite() && min_speedup_ratio > 0.0 {
        min_speedup_ratio
    } else {
        DEFAULT_WASM_CACHE_MIN_SPEEDUP_RATIO
    };

    let wasm_source = Path::new(wasm_path);
    if !wasm_source.exists() {
        return Err(format!("wasm artifact does not exist: {wasm_path}"));
    }
    let wasm_source = fs::canonicalize(wasm_source)
        .map_err(|error| format!("failed to canonicalize wasm artifact path: {error}"))?;
    let temp_root = std::env::temp_dir().join(format!(
        "loong-wasm-cache-benchmark-{}",
        current_epoch_seconds()
    ));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create benchmark temp directory: {error}"))?;

    let (benchmark_source, source_note) = {
        let metadata = fs::metadata(&wasm_source)
            .map_err(|error| format!("failed to read wasm metadata for benchmark: {error}"))?;
        if metadata.len() <= 8 {
            let synthetic_source = temp_root.join("synthetic-benchmark-module.wasm");
            write_synthetic_wasm_benchmark_module(&synthetic_source)?;
            (
                synthetic_source,
                Some(format!(
                    "input wasm `{}` appears to be a placeholder ({} bytes); using synthetic benchmark module with exported `run` function",
                    wasm_source.display(),
                    metadata.len()
                )),
            )
        } else {
            (wasm_source.clone(), None)
        }
    };

    let mut cold_latencies_ms = Vec::with_capacity(cold_iterations);
    let mut cold_cache_hits = 0usize;
    let mut cold_cache_misses = 0usize;
    for iteration in 0..cold_iterations {
        let candidate = temp_root.join(format!("cold-{iteration}.wasm"));
        fs::copy(&benchmark_source, &candidate)
            .map_err(|error| format!("failed to prepare cold benchmark artifact: {error}"))?;
        let sample = run_wasm_bridge_sample(&candidate)?;
        cold_latencies_ms.push(sample.latency_ms);
        if sample.cache_hit {
            cold_cache_hits = cold_cache_hits.saturating_add(1);
        } else {
            cold_cache_misses = cold_cache_misses.saturating_add(1);
        }
    }

    let hot_artifact = temp_root.join("hot.wasm");
    fs::copy(&benchmark_source, &hot_artifact)
        .map_err(|error| format!("failed to prepare hot benchmark artifact: {error}"))?;
    for _ in 0..warmup_iterations {
        let _ = run_wasm_bridge_sample(&hot_artifact)?;
    }

    let mut hot_latencies_ms = Vec::with_capacity(hot_iterations);
    let mut hot_cache_hits = 0usize;
    let mut hot_cache_misses = 0usize;
    for _ in 0..hot_iterations {
        let sample = run_wasm_bridge_sample(&hot_artifact)?;
        hot_latencies_ms.push(sample.latency_ms);
        if sample.cache_hit {
            hot_cache_hits = hot_cache_hits.saturating_add(1);
        } else {
            hot_cache_misses = hot_cache_misses.saturating_add(1);
        }
    }

    let cold_latency_ms = super::compute_numeric_stats(&cold_latencies_ms);
    let hot_latency_ms = super::compute_numeric_stats(&hot_latencies_ms);
    let observed_speedup_ratio = match (cold_latency_ms.p95, hot_latency_ms.p95) {
        (Some(cold_p95), Some(hot_p95)) if hot_p95 > 0.0 => Some(cold_p95 / hot_p95),
        _ => None,
    };

    let mut gate_reason = None;
    let gate_passed = if enforce_gate {
        match observed_speedup_ratio {
            Some(observed) if observed >= normalized_min_speedup_ratio => true,
            Some(observed) => {
                gate_reason = Some(format!(
                    "observed p95 speedup ratio {:.3} is below threshold {:.3}",
                    observed, normalized_min_speedup_ratio
                ));
                false
            }
            None => {
                gate_reason = Some("unable to compute p95 speedup ratio".to_owned());
                false
            }
        }
    } else {
        true
    };

    let report = WasmCacheBenchmarkReport {
        generated_at_epoch_s: current_epoch_seconds(),
        profile: "release".to_owned(),
        input_wasm_path: wasm_source.display().to_string(),
        effective_wasm_path: benchmark_source.display().to_string(),
        source_note: source_note.clone(),
        output_path: output_path.to_owned(),
        cold_iterations,
        hot_iterations,
        warmup_iterations,
        cold_latency_ms,
        hot_latency_ms,
        cold_cache_hits,
        cold_cache_misses,
        hot_cache_hits,
        hot_cache_misses,
        speedup_ratio_p95: observed_speedup_ratio,
        gate: WasmCacheBenchmarkGateSummary {
            enforced: enforce_gate,
            passed: gate_passed,
            min_speedup_ratio: normalized_min_speedup_ratio,
            observed_speedup_ratio,
            reason: gate_reason.clone(),
        },
    };

    write_json_file(output_path, &report)?;
    println!("wasm cache benchmark report written to {output_path}");
    println!(
        "cold p95={:.3}ms hot p95={:.3}ms speedup_ratio_p95={:.3} gate={}",
        report.cold_latency_ms.p95.unwrap_or(0.0),
        report.hot_latency_ms.p95.unwrap_or(0.0),
        report.speedup_ratio_p95.unwrap_or(0.0),
        if report.gate.passed { "pass" } else { "fail" }
    );
    println!(
        "cache cold(hit/miss)={}/{} hot(hit/miss)={}/{}",
        report.cold_cache_hits,
        report.cold_cache_misses,
        report.hot_cache_hits,
        report.hot_cache_misses
    );
    if let Some(note) = source_note {
        println!("note: {note}");
    }

    let _ = fs::remove_dir_all(&temp_root);

    if enforce_gate && !gate_passed {
        let reason = gate_reason.unwrap_or_else(|| "gate failed".to_owned());
        return Err(format!(
            "wasm cache benchmark regression gate failed: {reason}"
        ));
    }

    Ok(())
}

fn write_synthetic_wasm_benchmark_module(path: &Path) -> CliResult<()> {
    const SYNTHETIC_WASM_WITH_RUN_EXPORT: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03,
        0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x00, 0x0a, 0x04, 0x01,
        0x02, 0x00, 0x0b,
    ];
    fs::write(path, SYNTHETIC_WASM_WITH_RUN_EXPORT)
        .map_err(|error| format!("failed to write synthetic wasm benchmark module: {error}"))?;
    Ok(())
}

fn run_wasm_bridge_sample(wasm_artifact: &Path) -> CliResult<WasmBridgeSample> {
    let canonical_artifact = fs::canonicalize(wasm_artifact).map_err(|error| {
        format!(
            "failed to canonicalize benchmark wasm artifact {}: {error}",
            wasm_artifact.display()
        )
    })?;
    let artifact_parent = canonical_artifact.parent().ok_or_else(|| {
        format!(
            "failed to compute parent directory for {}",
            canonical_artifact.display()
        )
    })?;
    let provider = ProviderConfig {
        provider_id: "wasm-cache-benchmark-provider".to_owned(),
        connector_name: "wasm-cache-benchmark-provider".to_owned(),
        version: "0.1.0".to_owned(),
        metadata: BTreeMap::from([
            (
                "component_resolved_path".to_owned(),
                canonical_artifact.display().to_string(),
            ),
            ("entrypoint".to_owned(), "run".to_owned()),
        ]),
    };
    let channel = ChannelConfig {
        channel_id: "primary".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: canonical_artifact.display().to_string(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"benchmark":"wasm_cache"}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        execute_process_stdio: false,
        execute_http_json: false,
        execute_wasm_component: true,
        compatibility_matrix: BridgeSupportMatrix::default(),
        allowed_process_commands: BTreeSet::new(),
        bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
        wasm_allowed_path_prefixes: vec![artifact_parent.to_path_buf()],
        wasm_guest_readable_config_keys: BTreeSet::new(),
        wasm_max_component_bytes: Some(8 * 1024 * 1024),
        wasm_max_output_bytes: None,
        wasm_fuel_limit: Some(2_000_000),
        wasm_timeout_ms: None,
        wasm_require_hash_pin: false,
        wasm_required_sha256_by_plugin: BTreeMap::new(),
        enforce_execution_success: true,
    };

    let started_at = StdInstant::now();
    let execution =
        execute_wasm_component_bridge(json!({}), &provider, &channel, &command, &runtime_policy);
    let latency_ms = started_at.elapsed().as_secs_f64() * 1_000.0;
    let status = execution
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status != "executed" {
        let reason = execution
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown reason");
        return Err(format!(
            "wasm bridge benchmark execution failed for {}: status={status} reason={reason}",
            canonical_artifact.display()
        ));
    }

    let cache_hit = execution
        .get("runtime")
        .and_then(Value::as_object)
        .and_then(|runtime| runtime.get("cache_hit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(WasmBridgeSample {
        latency_ms,
        cache_hit,
    })
}

#[doc(hidden)]
pub fn copy_benchmark_file(source: &Path, destination: &Path) -> CliResult<()> {
    match benchmark_copy_strategy_from_env(std::env::var(BENCHMARK_COPY_STRATEGY_ENV).ok()) {
        #[cfg(target_os = "macos")]
        BenchmarkCopyStrategy::MacosCloneCp => {
            let clone_attempt = std::process::Command::new("/bin/cp")
                .arg("-c")
                .arg(source)
                .arg(destination)
                .output();
            if let Ok(output) = clone_attempt
                && output.status.success()
            {
                return Ok(());
            }

            if destination.exists() {
                let _ = fs::remove_file(destination);
            }
        }
        BenchmarkCopyStrategy::StableFsCopy => {}
    }

    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "copy benchmark file {} -> {} failed: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn benchmark_copy_strategy_from_env(raw_input: Option<String>) -> BenchmarkCopyStrategy {
    #[cfg(target_os = "macos")]
    {
        if raw_input
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("clone"))
        {
            return BenchmarkCopyStrategy::MacosCloneCp;
        }
    }

    BenchmarkCopyStrategy::StableFsCopy
}
