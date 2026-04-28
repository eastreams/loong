# Loong Examples

This directory contains execution specifications, plugin samples, benchmark configurations,
and security policy profiles for Loong.

## Directory Index

| Directory | Contents | Description |
|-----------|----------|-------------|
| `spec/` | 13 JSON spec files | Execution specifications for deterministic test scenarios |
| `plugins/` | Rust source | Plugin manifest examples for scanner-based hotplug |
| `plugins-wasm/` | Rust source + compiled `.wasm` | WASM plugin source and compiled binary |
| `plugins-process/` | Python script + manifest-first package | Process-based plugin examples, including a manifest-first native extension package |
| `benchmarks/` | 2 JSON files | Performance benchmark matrix and baseline configurations |
| `policy/` | 2 JSON files | Security policy profiles (approval, scanning) |

## Spec Files

Each spec file is a self-contained execution scenario. No external services required.

| File | Description |
|------|-------------|
| `runtime-extension.json` | Core/extension runtime adapter dispatch |
| `tool-search.json` | Tool discovery across providers and plugins |
| `tool-search-trusted.json` | Tool discovery constrained to explicit trusted plugin tiers |
| `tool-approval-per-call.json` | Per-call human approval gate |
| `programmatic-tool-call.json` | Server-side tool orchestration pipeline |
| `plugin-scan-hotplug.json` | Plugin scanning and hotplug lifecycle |
| `plugin-bootstrap-enforce.json` | Bootstrap enforcement gate |
| `plugin-bootstrap-trust-policy.json` | Explicit trust-policy gate for unverified high-risk plugin auto-apply |
| `plugin-bridge-enforce.json` | Bridge support matrix enforcement |
| `plugin-wasm-security-scan.json` | WASM plugin static analysis |
| `plugin-process-stdio-exec.json` | Process stdio bridge execution with trust-policy override left disabled |
| `auto-provider-hotplug.json` | Autonomous provider integration |
| `self-awareness-guard.json` | Architecture guard evaluation |

## Running Spec Files

```bash
loong run-spec --spec examples/spec/runtime-extension.json --print-audit
loong run-spec --spec examples/spec/tool-search-trusted.json --render-summary
```

`--print-audit` shows the full audit trail for the execution.
`--render-summary` writes a compact operator-facing summary to `stderr` while
preserving the full JSON report on `stdout`.

Generate a trust-guarded starter spec:

```bash
loong init-spec --preset plugin-trust-guard --output loong.plugin-trust.json
```

Run all spec files:

```bash
for spec in examples/spec/*.json; do
  echo "--- Running: $spec ---"
  loong run-spec --spec "$spec" --print-audit
done
```

## Running Benchmarks

Programmatic pressure benchmark:

```bash
loong benchmark-programmatic-pressure \
  --matrix examples/benchmarks/programmatic-pressure-matrix.json \
  --enforce-gate
```

WASM cache benchmark:

```bash
loong benchmark-wasm-cache \
  --wasm examples/plugins-wasm/secure_echo.wasm \
  --enforce-gate
```

Optional runtime tuning:

```bash
# default = 32, max = 4096
LOONG_WASM_CACHE_CAPACITY=64 loong benchmark-wasm-cache \
  --wasm examples/plugins-wasm/secure_echo.wasm \
  --enforce-gate
```

Convenience scripts:

```bash
./scripts/benchmark_programmatic_pressure.sh
./scripts/benchmark_wasm_cache.sh
```

## Plugin Examples

- `plugins/openrouter_plugin.rs` -- Rust plugin with embedded `LOONG_PLUGIN_START` / `LOONG_PLUGIN_END` manifest markers. This example is marked `verified-community` so tool search and catalog reports surface a non-default trust tier.
- `plugins-wasm/secure_wasm_plugin.rs` -- WASM plugin Rust source. Compiled to `secure_echo.wasm`, with an `official` trust tier for first-party runtime examples.
- `plugins-process/stdio_echo_plugin.py` -- Python stdio echo plugin for process-bridge testing. This example stays explicitly `unverified` so the trust-policy fixtures can demonstrate blocked auto-apply without hiding the plugin from scan/search results.
- `plugins-process/native-extension-python/` -- Minimal manifest-first Python native extension example with doctor/inventory/smoke-test commands.
- `plugins-process/native-extension-go/` -- Minimal manifest-first Go native extension example with the same governed authoring loop.
- `plugins-process/native-extension-rust/` -- Minimal manifest-first Rust native extension example with the same governed authoring loop.

Tool search and bootstrap reports now surface `trust_tier` and `provenance_summary` fields for scanned plugin-backed providers. `run-spec` reports also include a top-level `plugin_trust_summary`, so operator review can see tier counts and review-required high-risk plugins without manually diffing raw scan/bootstrap arrays. `tool_search` accepts both inline query prefixes like `trust:official` / `tier:verified-community` and a structured `trust_tiers` array on the operation payload when specs need deterministic trust-aware discovery. The `tool_search` outcome now also carries a `trust_filter_summary` block so operators can see which trust scope was requested, how many candidates were filtered out, and whether conflicting trust constraints collapsed the result set. For quick operator review, `run-spec` also mirrors this into a top-level `tool_search_summary` with the query, returned count, trust scope, and compact top-result cards, and `loong run-spec --render-summary` renders the same trust-aware summary to `stderr` without breaking JSON consumers. The audit lane now also records `ToolSearchEvaluated` events, so `loong audit recent`, `loong audit summary`, and the dedicated `loong audit discovery` subcommand can surface trust-filter conflicts and trust-filtered empty discovery outcomes directly, including trust-scope rollups, the last triage label, a compact context summary, and an operator hint for what to adjust next.

When auditing retained history, you can now narrow the view to the relevant trust-aware events directly:

```bash
loong audit recent --kind tool-search-evaluated --limit 5
loong audit recent --kind tool-search-evaluated --query-contains "trust:official" --trust-tier official
loong audit summary --triage-label tool-search-trust-conflict
loong audit summary --group-by token
loong audit discovery --query-contains "trust:official" --trust-tier official
loong audit discovery --group-by agent
loong audit discovery --since-epoch-s 1700010051 --until-epoch-s 1700010052 --query-contains "catalog"
loong audit recent --pack-id tool-search-trusted-pack --agent-id agent-tool-search-trusted
loong audit recent --event-id evt-tool-search-conflict --token-id token-tool-search
loong audit token-trail --token-id token-tool-search --limit 20
```

The `limit` applies after filtering, so these commands return the most recent matching events instead of trimming the full journal first. `audit discovery` also preloads the `ToolSearchEvaluated` kind filter and aggregates requested/effective trust tiers so operators can review trust-aware discovery drift without manually composing event-kind filters. When you need to isolate one rollout or incident window, `audit recent`, `audit summary`, and `audit discovery` also accept inclusive `--since-epoch-s` / `--until-epoch-s` filters and echo those bounds back in text and JSON output.
They also accept `--pack-id` and `--agent-id`, which is useful when retained history contains overlapping validation runs from multiple packs or operators. For exact retained drill-down, `--event-id` and `--token-id` can be layered on top of those broader filters; the token filter follows typed `TokenIssued`, `TokenRevoked`, and `AuthorizationDenied` events, so one credential incident can be reconstructed without ad-hoc journal grep.
`audit summary --group-by pack|agent|token` gives a grouped operator rollup over the same filtered window, which is useful when you need to see whether failures cluster around one workload, one operator session, or one token family before drilling into a specific trail.
`audit discovery --group-by pack|agent` does the same for trust-aware tool-search history, but keeps the trust-tier and triage rollups intact so you can see which pack or agent is producing the most trust-filter conflicts or empty discovery outcomes.
Each grouped discovery row also includes a `drill_down_command` that replays the same retained slice through `audit recent`, including `--query-contains` / `--trust-tier` when those filters were active, so operators can jump from hotspot rollups to exact event windows without rebuilding the command.
The same grouped row now also includes a `correlated_summary_command`, which broadens that hotspot into `audit summary` over the same workload identity and time bounds so adjacent failover, authorization, or bootstrap triage can be correlated quickly.
Grouped discovery output now also shows a compact correlated summary preview beside that command, so operators can see the widened event-kind and triage counts before they pivot.
That preview now also exposes a focused signal summary with `additional_events`, non-discovery counts, and an `attention_hint`, so adjacent audit trouble stands out immediately.
The same focused layer now also includes a `remediation_hint`, which maps the strongest adjacent triage or event family to the next operator action worth taking.
When that mapping is deterministic, grouped discovery now also emits a `correlated_remediation_command`, so operators can jump directly into the next filtered audit view instead of translating the hint by hand.
When you want that reconstruction as a first-class operator surface instead of an ad-hoc filtered window, `audit token-trail` renders the token lifecycle directly and reports when the retained view has been truncated by the selected limit.

For how to write new plugins, see [CONTRIBUTING.md](../CONTRIBUTING.md) recipes (Add a Provider, Add a Tool, Add a Channel).

## Security Policy Profiles

| File | Description |
|------|-------------|
| `policy/approval-medium-balanced.json` | Medium-balanced human approval profile. High-risk tool calls require explicit authorization; low-risk calls stay fast. |
| `policy/security-scan-medium-balanced.json` | Medium-balanced security scan profile with `block_on_high` gate. |

These profiles are loaded by the policy engine at runtime. See [Layered Kernel Design](../docs/design-docs/layered-kernel-design.md) L1 for policy semantics.
