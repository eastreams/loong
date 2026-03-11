<!-- logo placeholder: replace with actual logo when available -->
<!-- <p align="center"><img src="logo.png" alt="LoongClaw" width="200"/></p> -->

<h1 align="center">LoongClaw</h1>

<p align="center">
  <strong>A Rust-first Agentic OS foundation -- stable kernel contracts, strict policy boundaries, pluggable runtime orchestration.</strong>
</p>

<p align="center">
  <a href="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml"><img src="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/rust-edition%202021-orange.svg" alt="Rust Edition 2021" />
  <img src="https://img.shields.io/badge/version-0.1.2--pre-yellow.svg" alt="Version: 0.1.2-pre" />
</p>

<p align="center">
  <a href="https://x.com/loongclawai"><img src="https://img.shields.io/badge/Follow-loongclawai-000000?logo=x&logoColor=white" alt="X" /></a>
  <a href="https://t.me/loongclaw"><img src="https://img.shields.io/badge/Telegram-loongclaw-26A5E4?logo=telegram&logoColor=white" alt="Telegram" /></a>
  <a href="https://discord.gg/7kSTX9mca"><img src="https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white" alt="Discord" /></a>
  <a href="https://www.reddit.com/r/LoongClaw"><img src="https://img.shields.io/badge/Reddit-r%2Floongclaw-FF4500?logo=reddit&logoColor=white" alt="Reddit" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="#why-loongclaw">Why LoongClaw?</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#key-features">Features</a> •
  <a href="#architecture-overview">Architecture</a> •
  <a href="#contributing">Contributing</a>
</p>

---

## Why LoongClaw?

LoongClaw is a layered Agentic OS kernel focused on stable kernel contracts, strict policy boundaries, and pluggable runtime orchestration. Core and business logic are strictly separated:

- **Minimal, stable core** -- handles only policy, security, and audit. No business logic in the kernel.
- **Security cannot be bypassed** -- every tool call, memory operation, and connector invocation is gated by the policy engine. High-risk actions require explicit human authorization.
- **Business logic lives in extension planes** -- providers, tools, channels, and memory backends are all replaceable adapters that never touch the kernel.
- **Multi-language plugins** -- supports Rust, WASM, and process plugins in any language. The community can extend freely.
- **Bidirectional integration** -- can be embedded as a kernel into other systems, or connect to external services via adapters.

## Sponsors

<p align="center">
  <a href="https://www.volcengine.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/volcengine-logo-dark.png"/>
      <img src="assets/sponsors_logo/volcengine-logo-light.png" alt="Volcengine" height="48"/>
    </picture>
  </a>
  <br/><br/>
  Thanks to <a href="https://www.volcengine.com">Volcengine</a> for sponsoring this project.
</p>

## Quick Start

### Prerequisites

- Rust stable toolchain (edition 2021)
- `cargo` available in your PATH

### Install from Source

<details>
<summary>Linux / macOS</summary>

```bash
./scripts/install.sh --setup
```
</details>

<details>
<summary>Windows (PowerShell)</summary>

```powershell
pwsh ./scripts/install.ps1 -Setup
```
</details>

<details>
<summary>Manual (Cargo)</summary>

```bash
cargo install --path crates/daemon
```
</details>

### First Chat in Under 5 Minutes

1. Generate config and bootstrap local state:

   ```bash
   loongclaw setup
   ```

2. Set your provider API key:

   ```bash
   export PROVIDER_API_KEY=sk-...
   ```

3. Start chatting:

   ```bash
   loongclaw chat
   ```

Run `loongclaw doctor --fix` if anything goes wrong.

### Run Tests

```bash
cargo test --workspace --all-features
```

## Key Features

**Kernel and Security**
- Capability-based policy engine with token lifecycle (issue, revoke, authorize)
- Human approval gates: per-call authorization or one-time full-access mode
- Plugin security scanning with `block_on_high` hard gate
- WASM static analysis (artifact paths, module size, hash pin, import policy)
- External profile integrity: checksum pinning + ed25519 signature verification
- JSONL SIEM export lane with optional fail-closed mode
- Denylist precedence over all grants

**Runtime and Execution**
- Core/Extension adapter pattern for runtime, tool, memory, and connector planes
- WASM runtime execution via Wasmtime with policy-driven resource limits
- Process-stdio and HTTP-JSON bridge lanes with protocol authorization
- Programmatic tool orchestration with batching, retry, circuit breakers, and adaptive concurrency
- Tool discovery across providers and scanned plugin descriptors

**MVP Product Layer**
- `setup` -- generate TOML config and bootstrap SQLite memory
- `onboard` -- guided first-run with preflight diagnostics
- `doctor` -- diagnostics with optional safe fixes (`--fix`) and machine-readable output (`--json`)
- `chat` -- interactive CLI with sliding-window conversation memory
- Core tools: `shell.exec`, `file.read`, `file.write`
- Providers: OpenAI-compatible, Volcengine custom endpoint
- Channels: CLI, Telegram polling, Feishu encrypted webhook

**Protocol Foundation**
- Typed transport contracts and protocol method routing
- JSON-line stream transport for stdio/pipe integration
- Linked in-memory channel transport with bounded backpressure
- Route authorization gates before handler dispatch

**Developer Experience**
- 7-crate DAG with zero cycles and strict dependency direction
- 370+ tests with strict lint/fmt CI gates at every commit
- Cargo feature flags for modular builds
- Spec-driven deterministic test execution
- Benchmark gates for programmatic pressure and WASM cache performance

## Architecture Overview

LoongClaw is organized as a 7-crate workspace with a strict dependency DAG:

```text
contracts (leaf -- zero internal deps)
├── kernel --> contracts
├── protocol (independent leaf)
├── app --> contracts, kernel
├── spec --> contracts, kernel, protocol
├── bench --> contracts, kernel, spec
└── daemon (binary) --> all of the above
```

| Crate | Role |
|-------|------|
| `contracts` | Shared types, capability model. Zero deps -- the stable ABI surface. |
| `kernel` | Policy engine, audit timeline, capability tokens, plugin system, integration catalog, pack boundaries. |
| `protocol` | Transport contracts, typed routing. Independent leaf. |
| `app` | Providers, tools, channels, memory, configuration, conversation engine. |
| `spec` | Execution spec runner for deterministic test scenarios. |
| `bench` | Benchmark harness and gate enforcement. |
| `daemon` | CLI binary (`loongclaw`). Wires everything into runnable commands. |

For the full layered execution model (L0-L9), see [ARCHITECTURE.md](ARCHITECTURE.md).

## Feature Flags

All flags are enabled by default via the `mvp` meta-feature. You can disable defaults and
enable only what you need for minimal builds.

| Flag | Description |
|------|-------------|
| `config-toml` | TOML configuration loader |
| `memory-sqlite` | SQLite conversation memory |
| `tool-shell` | `shell.exec` tool |
| `tool-file` | `file.read` / `file.write` tools |
| `channel-cli` | Interactive CLI channel |
| `channel-telegram` | Telegram polling adapter |
| `channel-feishu` | Feishu encrypted webhook adapter |
| `provider-openai` | OpenAI-compatible provider |
| `provider-volcengine` | Volcengine custom endpoint |

Example minimal build:

```bash
cargo build -p loongclaw-daemon --no-default-features --features "channel-cli,provider-openai,config-toml,memory-sqlite"
```

## Design Principles

1. **Kernel-first** -- all execution paths route through the kernel's capability, policy, and audit system. No shadow paths.
2. **No breaking changes** -- new features are additive only. Existing public API signatures stay unchanged.
3. **Capability-gated by default** -- every operation requires a valid `CapabilityToken` with matching capabilities.
4. **Audit everything security-critical** -- policy denials, token lifecycle events, and module invocations all emit structured events.
5. **7-crate DAG, no cycles** -- dependency direction is non-negotiable.
6. **Tests first** -- if a behavior isn't tested, it doesn't exist.
7. **Proven technology preferred** -- choose well-understood, composable dependencies over opaque packages.
8. **Repository is the system of record** -- if it's not in the repo, it doesn't exist for agents.
9. **Automate first** -- linters, CI gates, and pre-commit hooks over code review comments.
10. **Strictly avoid over-engineering** -- minimum complexity for the current task is the right amount.

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture](ARCHITECTURE.md) | Crate structure and layered execution model |
| [Core Beliefs](docs/design-docs/core-beliefs.md) | 10 core engineering principles |
| [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) | Full L0-L9 layer specification |
| [Roadmap](docs/roadmap.md) | Stage-based milestones and acceptance criteria |
| [Reliability](docs/RELIABILITY.md) | Build and kernel invariants |
| [Examples](examples/README.md) | Spec files, plugin samples, benchmarks |
| [Product Specs](docs/product-specs/index.md) | User-facing requirements (in progress) |
| [Changelog](CHANGELOG.md) | Release history |

## Configuration

`loongclaw setup` defaults to referencing secrets via environment variables (not storing them directly):

```toml
[provider]
kind = "openai"
api_key_env = "PROVIDER_API_KEY"   # env var name, not the key itself
```

For direct values, use the non-`_env` fields instead (`api_key = "sk-..."`).

Validate your config:

```bash
loongclaw validate-config --config ~/.loongclaw/config.toml --json
```

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full workflow,
including contribution workflows (routine vs. higher-risk changes) and recipes for adding
providers, tools, and channels.

- [Contributing Guide](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## License

LoongClaw is licensed under the [MIT License](LICENSE-MIT).

Copyright (c) 2026 LoongClaw AI

## Star History

<p align="center">
  <a href="https://star-history.com/#loongclaw-ai/loongclaw&Date">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=loongclaw-ai/loongclaw&type=Date&theme=dark"/>
      <img src="https://api.star-history.com/svg?repos=loongclaw-ai/loongclaw&type=Date" alt="Star History Chart"/>
    </picture>
  </a>
</p>
