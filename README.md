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

## MVP Foundation (In Progress)

- `setup` command: generate beginner-friendly TOML config and bootstrap SQLite memory.
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
cargo run -p loongclaw-daemon --bin loongclawd -- setup --force
cargo run -p loongclaw-daemon --bin loongclawd -- list-models --json
cargo run -p loongclaw-daemon --bin loongclawd -- chat
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/runtime-extension.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/tool-search.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/programmatic-tool-call.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- benchmark-programmatic-pressure --matrix examples/benchmarks/programmatic-pressure-matrix.json --enforce-gate
./scripts/benchmark_programmatic_pressure.sh
```

One-command install from source:

```bash
./scripts/install.sh --setup
```

PowerShell:

```powershell
pwsh ./scripts/install.ps1 -Setup
```

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
