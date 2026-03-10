# LoongClaw

LoongClaw is a Rust-first Agentic OS foundation focused on stable kernel contracts,
strict policy boundaries, and highly pluggable runtime orchestration.

## Workspace Layout

- `crates/kernel` (`loongclaw-kernel`): core architecture contracts and execution kernel.
- `crates/daemon` (`loongclaw-daemon` / `loongclawd`): runnable daemon wired to kernel policy and runtime controls.

## Core Design

The kernel enforces layered execution planes with core/extension separation:

- pack/policy boundaries
- harness runtime routing
- runtime/tool/memory/connector planes
- audit and deterministic timeline controls
- integration, plugin IR, bootstrap activation, architecture guard, and awareness snapshots

For architecture boundaries, see [Layered Kernel Design](docs/design-docs/layered-kernel-design.md).

## Current Validation Status

- `loongclaw-kernel`: 41 unit tests passing.
- `loongclaw-daemon`: 135 unit tests passing.
- `loongclawd` smoke/spec execution verified.
- `programmatic` pressure benchmark gate (matrix + baseline) verified.
- `wasm` cache benchmark gate (cold/hot latency + hit/miss) verified.

## MVP Foundation (In Progress)

- `setup` command: generate beginner-friendly TOML config and bootstrap SQLite memory.
- `onboard` command: guided first-run onboarding with preflight diagnostics and risk acknowledgement.
- `doctor` command: setup diagnostics with optional safe fixes (`--fix`) and machine-readable output (`--json`).
- `chat` command: interactive CLI channel with sliding-window conversation memory.
- Core tool runtime now supports:
  - `shell.exec`
  - `file.read`
  - `file.write`
- Provider config supports:
  - OpenAI-compatible endpoint composition
  - Volcengine custom endpoint mode
- Cargo feature flags are available for modular packaging:
  - `config-toml`, `memory-sqlite`, `tool-shell`, `tool-file`
  - channels: `channel-cli`, `channel-telegram`, `channel-feishu`
  - providers: `provider-openai`, `provider-volcengine`

## Quick Start

```bash
cargo test -p loongclaw-kernel
cargo test -p loongclaw-daemon
cargo run -p loongclaw-daemon --bin loongclawd
cargo run -p loongclaw-daemon --bin loongclawd -- onboard
cargo run -p loongclaw-daemon --bin loongclawd -- setup --force
cargo run -p loongclaw-daemon --bin loongclawd -- doctor --fix
cargo run -p loongclaw-daemon --bin loongclawd -- list-models --json
cargo run -p loongclaw-daemon --bin loongclawd -- chat
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/runtime-extension.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/tool-search.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/programmatic-tool-call.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- benchmark-programmatic-pressure --matrix examples/benchmarks/programmatic-pressure-matrix.json --enforce-gate
cargo run -p loongclaw-daemon --bin loongclawd -- benchmark-wasm-cache --wasm examples/plugins-wasm/secure_echo.wasm --enforce-gate
./scripts/benchmark_programmatic_pressure.sh
./scripts/benchmark_wasm_cache.sh
```

Optional runtime tuning:

```bash
# default = 32, max = 4096
LOONGCLAW_WASM_CACHE_CAPACITY=64 cargo run -p loongclaw-daemon --bin loongclawd -- benchmark-wasm-cache --enforce-gate
```

One-command install from source:

```bash
./scripts/install.sh --setup
```

PowerShell:

```powershell
pwsh ./scripts/install.ps1 -Setup
```

## Secret Config Quick Guide

`setup` defaults to environment-pointer mode:

- `provider.api_key_env` stores an env var name (for example `PROVIDER_API_KEY`).
- `telegram.bot_token_env` stores an env var name (for example `TELEGRAM_BOT_TOKEN`).

Do not place raw secrets in `*_env` fields.
Do not use shell wrappers in `*_env` fields (`$VAR`, `${VAR}`, or `%VAR%`).

If you need direct values in config, use the non-`_env` fields:

- `provider.api_key = "sk-..."`
- `telegram.bot_token = "..."`

Provider examples:

```toml
[provider]
kind = "kimi"
api_key_env = "MOONSHOT_API_KEY"
```

```toml
[provider]
kind = "minimax"
api_key_env = "MINIMAX_API_KEY"
```

Validate config before runtime startup:

```bash
loongclawd validate-config --config ~/.loongclaw/config.toml --json --locale en
```

`--json` returns stable diagnostic codes and machine-readable message variables
for downstream localization workflows.
Current builds ship an English diagnostic catalog (`en`) and normalize locale
aliases (for example `en-US`) to `en`.
JSON output includes:

- `diagnostics_schema_version` for contract evolution.
- `title_key` and `message_key` for i18n-friendly client rendering.
- `supported_locales` to advertise available catalogs.

CI gate example:

```bash
loongclawd validate-config --config ~/.loongclaw/config.toml --output problem-json --fail-on-diagnostics
```

`--fail-on-diagnostics` exits non-zero when diagnostics are present.

## Documentation Index

- [Core Beliefs](docs/design-docs/core-beliefs.md)
- [Layered Kernel Design](docs/design-docs/layered-kernel-design.md)
- [Roadmap](docs/roadmap.md)
- [Reliability](docs/RELIABILITY.md)
- [Product Specs](docs/product-specs/index.md)
- [Contributing Guide](CONTRIBUTING.md)

## Open Source Contribution

- [Contributing Guide](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)
