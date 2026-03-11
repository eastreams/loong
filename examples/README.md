# LoongClaw Examples

This directory contains execution specifications, plugin samples, benchmark configurations,
and security policy profiles for LoongClaw.

## Directory Index

| Directory | Contents | Description |
|-----------|----------|-------------|
| `spec/` | 11 JSON spec files | Execution specifications for deterministic test scenarios |
| `plugins/` | Rust source | Plugin manifest examples for scanner-based hotplug |
| `plugins-wasm/` | Rust source + compiled `.wasm` | WASM plugin source and compiled binary |
| `plugins-process/` | Python script | Process-based plugin example (stdio bridge) |
| `benchmarks/` | 2 JSON files | Performance benchmark matrix and baseline configurations |
| `policy/` | 2 JSON files | Security policy profiles (approval, scanning) |

## Spec Files

Each spec file is a self-contained execution scenario. No external services required.

| File | Description |
|------|-------------|
| `runtime-extension.json` | Core/extension runtime adapter dispatch |
| `tool-search.json` | Tool discovery across providers and plugins |
| `tool-approval-per-call.json` | Per-call human approval gate |
| `programmatic-tool-call.json` | Server-side tool orchestration pipeline |
| `plugin-scan-hotplug.json` | Plugin scanning and hotplug lifecycle |
| `plugin-bootstrap-enforce.json` | Bootstrap enforcement gate |
| `plugin-bridge-enforce.json` | Bridge support matrix enforcement |
| `plugin-wasm-security-scan.json` | WASM plugin static analysis |
| `plugin-process-stdio-exec.json` | Process stdio bridge execution |
| `auto-provider-hotplug.json` | Autonomous provider integration |
| `self-awareness-guard.json` | Architecture guard evaluation |

## Running Spec Files

```bash
loongclaw run-spec --spec examples/spec/runtime-extension.json --print-audit
```

`--print-audit` shows the full audit trail for the execution.

Run all spec files:

```bash
for spec in examples/spec/*.json; do
  echo "--- Running: $spec ---"
  loongclaw run-spec --spec "$spec" --print-audit
done
```

## Running Benchmarks

Programmatic pressure benchmark:

```bash
loongclaw benchmark-programmatic-pressure \
  --matrix examples/benchmarks/programmatic-pressure-matrix.json \
  --enforce-gate
```

WASM cache benchmark:

```bash
loongclaw benchmark-wasm-cache \
  --wasm examples/plugins-wasm/secure_echo.wasm \
  --enforce-gate
```

Optional runtime tuning:

```bash
# default = 32, max = 4096
LOONGCLAW_WASM_CACHE_CAPACITY=64 loongclaw benchmark-wasm-cache \
  --wasm examples/plugins-wasm/secure_echo.wasm \
  --enforce-gate
```

Convenience scripts:

```bash
./scripts/benchmark_programmatic_pressure.sh
./scripts/benchmark_wasm_cache.sh
```

## Plugin Examples

- `plugins/openrouter_plugin.rs` -- Rust plugin with embedded `LOONGCLAW_PLUGIN_START` / `LOONGCLAW_PLUGIN_END` manifest markers. The plugin scanner extracts these markers for hotplug integration.
- `plugins-wasm/secure_wasm_plugin.rs` -- WASM plugin Rust source. Compiled to `secure_echo.wasm`.
- `plugins-process/stdio_echo_plugin.py` -- Python stdio echo plugin for process-bridge testing.

For how to write new plugins, see [CONTRIBUTING.md](../CONTRIBUTING.md) recipes (Add a Provider, Add a Tool, Add a Channel).

## Security Policy Profiles

| File | Description |
|------|-------------|
| `policy/approval-medium-balanced.json` | Medium-balanced human approval profile. High-risk tool calls require explicit authorization; low-risk calls stay fast. |
| `policy/security-scan-medium-balanced.json` | Medium-balanced security scan profile with `block_on_high` gate. |

These profiles are loaded by the policy engine at runtime. See [Layered Kernel Design](../docs/design-docs/layered-kernel-design.md) L1 for policy semantics.
