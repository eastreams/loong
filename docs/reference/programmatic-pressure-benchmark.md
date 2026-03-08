# Programmatic Pressure Benchmark

This benchmark suite raises `programmatic_tool_call` validation from unit-level checks
into a near-production timing pressure gate.

## Scope

The suite validates the coupled behavior of:

- circuit breaker (`open -> half_open -> closed`)
- adaptive concurrency budget (`downshift -> recovery upshift`)
- connector-level rate shaping (`min_interval_ms`)

## Files

- matrix: [`examples/benchmarks/programmatic-pressure-matrix.json`](../../examples/benchmarks/programmatic-pressure-matrix.json)
- baseline thresholds: [`examples/benchmarks/programmatic-pressure-baseline.json`](../../examples/benchmarks/programmatic-pressure-baseline.json)
- output report (default): `target/benchmarks/programmatic-pressure-report.json`

## Scenario Matrix

Each scenario is declarative and supports two kinds:

- `spec_run`: executes a full `RunnerSpec` using `execute_spec`
- `circuit_half_open`: directly exercises circuit runtime transitions with timing probes

Per-scenario controls:

- `iterations` and `warmup_iterations`
- `allow_blocked` and `expected_operation_kind`
- scenario-specific payload/policy for failure injection and transition pressure

## Regression Gate

The baseline file defines threshold checks per scenario.

Supported checks:

- `max_error_rate`
- `max_p95_latency_ms`
- `max_p99_latency_ms`
- `min_throughput_rps`
- `min_peak_in_flight`
- `max_circuit_open_error_ratio`
- `max_half_open_p95_ms`
- `expected_schema_fingerprint` (step output schema contract)

Per-scenario drift strategy (`tolerance`) is also supported:

- `max_ratio`: relaxes all max-threshold checks by ratio
- `min_ratio`: relaxes all min-threshold checks by ratio
- `latency_ms`: additive headroom for latency checks (`p95/p99`)

Gate behavior:

- pass: all configured checks pass per scenario
- fail: any configured check fails
- strict mode (`--enforce-gate`): missing baseline for a scenario fails the run

Schema fingerprint behavior:

- Each `spec_run` scenario derives a deterministic fingerprint from `outcome.step_outputs` schema.
- The hash is shape-based (type/schema), not value-based, so dynamic payload values do not cause drift.
- Baseline `expected_schema_fingerprint` enforces schema compatibility across refactors.
- If multiple schema variants appear across iterations, the report emits a `multi:<sha256>` aggregate fingerprint.

### Drift Example

```json
{
  "max_p95_latency_ms": 600,
  "min_throughput_rps": 12.0,
  "tolerance": {
    "max_ratio": 0.15,
    "min_ratio": 0.10,
    "latency_ms": 20.0
  }
}
```

Effective gate semantics:

- max check: `observed <= base * (1 + max_ratio) + latency_ms?`
- min check: `observed >= base * (1 - min_ratio)`

## Commands

Run benchmark and emit report:

```bash
cargo run -p loongclaw-daemon -- benchmark-programmatic-pressure \
  --matrix examples/benchmarks/programmatic-pressure-matrix.json \
  --output target/benchmarks/programmatic-pressure-report.json
```

Run with regression enforcement:

```bash
cargo run -p loongclaw-daemon -- benchmark-programmatic-pressure \
  --matrix examples/benchmarks/programmatic-pressure-matrix.json \
  --baseline examples/benchmarks/programmatic-pressure-baseline.json \
  --enforce-gate \
  --output target/benchmarks/programmatic-pressure-report.json
```

Run via unified helper script:

```bash
./scripts/benchmark_programmatic_pressure.sh
```

## Report Highlights

The report includes:

- per-scenario latency stats (`p50/p95/p99`)
- throughput (`connector_calls_total / total_duration`)
- scheduler aggregates (`peak_in_flight`, budget reductions/increases, wait cycles)
- circuit transition timing (`half_open_transition_ms`)
- per-scenario `schema_fingerprint` for contract drift detection
- structured gate checks and pass/fail status

Use the report as the machine-readable artifact for performance regression audits.
