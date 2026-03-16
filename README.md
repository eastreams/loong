<!-- logo placeholder: replace with actual logo when available -->
<!-- <p align="center"><img src="logo.png" alt="LoongClaw" width="200"/></p> -->

<h1 align="center">LoongClaw</h1>

<p align="center">
  <strong>A Rust-first private assistant runtime: guided onboarding, one-shot ask, repair-first diagnostics, and safe extensible tools on top of a stable Agentic OS foundation.</strong>
</p>

<p align="center">
  <a href="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml"><img src="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/rust-edition%202024-orange.svg" alt="Rust Edition 2024" />
  <img src="https://img.shields.io/badge/version-0.1.2-yellow.svg" alt="Version: 0.1.2" />
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

LoongClaw is a layered Agentic OS runtime built to feel like a trustworthy private assistant first and an extensible platform second. Core and business logic are strictly separated:

- **Minimal, stable core** -- handles only policy, security, and audit. No business logic in the kernel.
- **Security cannot be bypassed** -- every tool call, memory operation, and connector invocation is gated by the policy engine. High-risk actions require explicit human authorization.
- **Business logic lives in extension planes** -- providers, tools, channels, and memory backends are all replaceable adapters that never touch the kernel.
- **Multi-language plugins** -- supports Rust, WASM, and process plugins in any language. The community can extend freely.
- **Bidirectional integration** -- can be embedded as a kernel into other systems, or connect to external services via adapters.
- **Operator-ready product layer** -- `onboard`, `ask`, `chat`, `doctor`, personalities, memory profiles, and legacy claw import are first-class runtime capabilities.

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

## Alpha-Test Highlights

- `onboard` is the default first-run flow for provider, memory, and channel-ready setup.
- `ask` gives users a one-shot assistant command for first success without entering a REPL.
- `chat` provides an interactive CLI channel with sliding-window conversation memory.
- `doctor` and `doctor --fix` are the explicit repair path when the local runtime is unhealthy.
- Core tool runtime now ships bounded browser automation (`browser.open`, `browser.extract`, `browser.click`) plus `web.fetch`, `shell.exec`, `file.read`, and `file.write`.
- Runtime tool catalogs now stay truthful to the active config: browser/web tools disappear when disabled, and external-skills lifecycle tools are only advertised when that runtime is enabled.
- Shipped assistant surfaces today are CLI first, with Telegram polling and Feishu webhook as optional channels after the base setup is healthy.
- Memory-system selection is now a stable builtin-only seam:
  - config: `[memory] system = "builtin"`
  - env: `LOONGCLAW_MEMORY_SYSTEM=builtin`
  - the runtime keeps LoongClaw-owned canonical history and reserves concrete external adapters for
    later dedicated tracks
- Conversation runtime now exposes a pluggable `context engine` seam with explicit lifecycle hooks
  (`bootstrap`, `ingest`, `assemble`, `after_turn`, `compact_context`) plus reserved subagent
  hooks for future multi-agent orchestration.
- Context assembly now carries richer metadata (`messages`, optional `estimated_tokens`, optional
  `system_prompt_addition`) so policy-driven prompt shaping and compaction can evolve without
  breaking the trait surface.
- Context engine selection supports config and env override:
  - config: `[conversation] context_engine = "default|legacy|<custom_id>"`
  - env: `LOONGCLAW_CONTEXT_ENGINE=<engine_id>`
- ACP is modeled as a separate control plane instead of being folded into provider turns or context
  assembly.
- Built-in `acpx` backend now supports session lifecycle, turn execution, cancellation, status
  inspection, config patching, doctor diagnostics, and backend-local MCP server injection.
- ACP agent selection is now an explicit control-plane policy instead of a backend heuristic:
  - config: `[acp] default_agent = "codex"`
  - config: `[acp] allowed_agents = ["codex", "claude"]`
  - conversation routes now derive `session_key = agent:<selected_agent>:<session_id>` and reject
    disallowed agent prefixes early.
- ACP dispatch is now a separate policy seam instead of being implied by `[acp].enabled`:
  - config: `[acp.dispatch] enabled = true`
  - config: `[acp.dispatch] conversation_routing = "all"|"agent_prefixed_only"`
  - config: `[acp.dispatch] allowed_channels = ["telegram", "feishu"]`
  - config: `[acp.dispatch] allowed_account_ids = ["work-bot", "lark-prod"]`
  - config: `[acp.dispatch] thread_routing = "all"|"thread_only"|"root_only"`
  - this keeps “ACP control plane exists” separate from “which conversation turns default into ACP”
    so mixed provider/ACP operation and future thread binding do not require a route-layer rewrite.
  - channel filtering is evaluated against the underlying conversation route, even when the session
    is already agent-prefixed.
  - account filtering and thread/root filtering are evaluated against the typed conversation
    address (`channel/account/conversation/thread`) when available, then fall back to legacy
    `session_id` parsing for compatibility.
- Channel-originated turns now pass a typed session address (`channel/account/conversation/thread`)
  into ACP dispatch before any legacy `session_id` parsing, pre-embedding future account/thread
  binding rules without changing the public conversation/runtime seams again.
- ACP session bindings now persist a typed `binding_route_session_id` in addition to legacy
  `conversation_id`, so future account/thread-scoped ACP reuse does not depend on opaque aliases.
- ACP bootstrap now also carries an explicit typed binding scope into the control plane, so session
  reuse does not depend on re-parsing metadata alone.
- When `[acp].enabled = true` and ACP dispatch allows the session, CLI/channel turns route through
  the ACP manager with stable `conversation_id` and derived `session_key`, pre-wiring future
  persistent bindings and per-channel ACP routing without a conversation-runtime rewrite.
- When `[acp].emit_runtime_events = true`, ACP-routed turns persist structured
  `acp_turn_event` / `acp_turn_final` records into conversation history so daemon-side summaries
  and future OpenClaw-style streaming or telemetry surfaces can evolve without changing the ACP
  manager/backend seam again. Those persisted records now also carry explicit `agent_id`, so
  observability does not need to reverse-engineer identity only from `session_key`. They also keep
  `routing_intent` / `routing_origin`, while ACP session status surfaces keep
  `activation_origin`, so operators can distinguish explicit ACP entry from automatic ACP routing.
- The daemon now exposes operator-facing diagnostics for:
  - `list-context-engines`
  - `list-memory-systems`
  - `list-acp-backends`
  - `list-acp-sessions`
  - `acp-doctor`
  - `acp-dispatch`
  - `acp-event-summary`
  - `acp-status`
  - `acp-observability`

  `acp-dispatch` now reports not only whether automatic ACP routing is allowed, but also the
  predicted automatic routing origin (`automatic_agent_prefixed` vs `automatic_dispatch`) when the
  session would enter ACP.

### Runtime Introspection Commands

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- list-models --json
cargo run -p loongclaw-daemon --bin loongclawd -- list-context-engines --json
cargo run -p loongclaw-daemon --bin loongclawd -- list-memory-systems --json
cargo run -p loongclaw-daemon --bin loongclawd -- list-acp-backends --json
cargo run -p loongclaw-daemon --bin loongclawd -- list-acp-sessions --json
cargo run -p loongclaw-daemon --bin loongclawd -- acp-doctor --backend acpx --json
cargo run -p loongclaw-daemon --bin loongclawd -- acp-dispatch --session opaque-session --channel feishu --conversation-id oc_123 --account-id lark-prod --thread-id om_thread_1 --json
cargo run -p loongclaw-daemon --bin loongclawd -- acp-event-summary --session default --json
cargo run -p loongclaw-daemon --bin loongclawd -- acp-observability --json
# if an ACP session already exists:
# cargo run -p loongclaw-daemon --bin loongclawd -- acp-status --conversation-id telegram:42 --json
# cargo run -p loongclaw-daemon --bin loongclawd -- acp-status --route-session-id feishu:lark-prod:oc_123:om_thread_1 --json
# optional ACP runtime-event persistence for summaries / future streaming:
# [acp]
# enabled = true
# default_agent = "codex"
# allowed_agents = ["codex", "claude"]
# emit_runtime_events = true
# [acp.dispatch]
# enabled = true
# conversation_routing = "all"
# allowed_channels = ["telegram"]
# allowed_account_ids = ["work-bot"]
# thread_routing = "all"
# optional env override demo:
# LOONGCLAW_CONTEXT_ENGINE=legacy cargo run -p loongclaw-daemon --bin loongclawd -- list-context-engines --json
```

## Quick Start

### Install Script (Release-First When Available)

The bootstrap installer is fetched directly from the repository. It prefers the matching GitHub
Release binary, verifies its SHA256 checksum, installs `loongclaw`, and can immediately hand you
into guided onboarding.

If the repository has not published its first release yet, the installer exits with a clear message.
Use the source install path below in that case.

<details>
<summary>Linux / macOS</summary>

```bash
curl -fsSL https://raw.githubusercontent.com/loongclaw-ai/loongclaw/main/scripts/install.sh | bash -s -- --onboard
```
</details>

<details>
<summary>Windows (PowerShell)</summary>

```powershell
$script = Join-Path $env:TEMP "loongclaw-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/loongclaw-ai/loongclaw/main/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```
</details>

Available installer options:

- `--onboard` / `-Onboard` runs `loongclaw onboard` after install.
- `--version <tag>` / `-Version <tag>` installs a specific release instead of `latest`.
- `--source` / `-Source` falls back to building from a local repository checkout.
- `--prefix <dir>` / `-Prefix <dir>` changes the install directory.

### Build from Source

Prerequisites:

- Rust stable toolchain (edition 2024)
- `cargo` available in your PATH

<details>
<summary>Linux / macOS</summary>

```bash
bash scripts/install.sh --source --onboard
```
</details>

<details>
<summary>Windows (PowerShell)</summary>

```powershell
pwsh ./scripts/install.ps1 -Source -Onboard
```
</details>

<details>
<summary>Manual (Cargo)</summary>

```bash
cargo install --path crates/daemon
```
</details>

`--onboard` runs `loongclaw onboard` without `--force`, so rerunning this quickstart will stop before overwriting an existing config.

### First Answer in Under 5 Minutes

1. Run guided onboarding:

   ```bash
   loongclaw onboard
   ```

2. When onboarding asks for a credential env var, enter the env-var name only
   such as `OPENAI_API_KEY`, then export the real secret:

   ```bash
   export OPENAI_API_KEY=sk-...
   ```

3. Get a first one-shot answer:

   ```bash
   loongclaw ask --message "Summarize this repository and suggest the best next step."
   ```

   On a healthy setup, onboarding and `doctor` now print this style of ask example directly so the
   first success path is visible without reading docs first.

4. Continue with interactive chat when you want to stay in session:

   ```bash
   loongclaw chat
   ```

   Use `loongclaw chat --acp` when you want this chat session to route turns through ACP
   explicitly. Without `--acp` or other ACP-specific chat flags, normal chat stays on the default
   provider/context-engine path. The chat banner now starts with a concrete first prompt and keeps
   the ACP/runtime context in a compact operator-readable block.

Run `loongclaw doctor --fix` if anything goes wrong, or when onboarding / ask / chat reports a
local health issue. `doctor` now prints next actions such as credential env hints, safe repair
commands, and ask/chat follow-ups instead of only raw status lines.

### Run Tests

```bash
cargo test --workspace --all-features
```

## Prompt And Personality

LoongClaw ships with a native prompt pack and three default personalities. All
three personalities keep the same security-first boundaries; they only change
tone, initiative, confirmation style, and response density.

- `calm_engineering`: rigorous, direct, and technically grounded
- `friendly_collab`: warm, cooperative, and explanatory when helpful
- `autonomous_executor`: decisive, high-initiative, and execution-oriented

Interactive onboarding now walks through provider, model, credential env var,
personality, memory profile, and a final review step without dumping the raw
prompt text onto the screen. Advanced operators can still pass
`--system-prompt` for a full inline override.

## Memory Profiles

LoongClaw separates memory behavior from the storage backend. The current
backend is SQLite, with three operator-selectable context injection modes:

- `window_only`: only the recent sliding window is loaded
- `window_plus_summary`: earlier turns are condensed into a summary block
- `profile_plus_window`: a durable `profile_note` block is injected before the recent window

`profile_note` is the first migration-friendly durable memory lane. It is meant
to carry imported claw identity, stable preferences, or long-lived operator
tuning without forcing everything into the system prompt. Interactive onboarding
includes a dedicated memory-profile step and mirrors the selected profile in the
review and success summaries.

## Memory Systems

LoongClaw now treats `memory.system` as a stable selection seam, but the current
runtime surface remains intentionally builtin-only:

- `builtin` keeps canonical raw conversation history, typed canonical records,
  and deterministic prompt hydration inside LoongClaw.
- Future external memory systems are expected to plug in below final prompt
  projection, not replace LoongClaw's context authority.
- Memory-system failures are designed to fail open, preserving the baseline
  recent-window chat experience instead of turning memory into a hidden
  availability dependency.

Use the runtime diagnostics command below to inspect the selected system,
capability set, memory profile, ingest mode, and effective fail-open policy:

```bash
loongclaw list-memory-systems --json
```

## Migration And Import

LoongClaw can discover legacy claw homes during onboarding and offer an import
before the rest of onboarding continues.

- Recommended path: import a single highest-confidence source.
- Advanced path: plan multiple sources, merge only the profile lane, and keep prompt/system identity single-source.
- Safety defaults: secrets are not migrated, imported runtime identity is normalized to `LoongClaw`, and every apply creates a backup manifest with rollback support.

CLI migration workflow:

- Default mode is now `plan` (safe preview, no file write) when `--mode` is omitted.
- `apply_selected` accepts both `--source-id` and alias `--selection-id`.
- Safe merge accepts both `--primary-source-id` and alias `--primary-selection-id`.
- `map_external_skills` builds a deterministic external-skills mapping plan.
- `--apply-external-skills-plan` can attach that mapping into `profile_note` during `apply_selected`.
- applying external-skills plan also writes `.loongclaw-migration/<config>.external-skills.json` for audit and replay.

```bash
# Discover and score import candidates under a root
loongclaw import-claw --mode discover --input ~/legacy-claws

# Plan all candidates and print recommendation
loongclaw import-claw --mode plan_many --input ~/legacy-claws

# Preview external skills mapping artifacts and generated profile addendum
loongclaw import-claw --mode map_external_skills --input ~/legacy-claws

# Apply one selected source to a target config
loongclaw import-claw --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml --force

# Apply selected source and also attach external-skills mapping addendum
loongclaw import-claw --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml \
  --apply-external-skills-plan --force

# Roll back the last apply_selected/import apply for this output config
loongclaw import-claw --mode rollback_last_apply --output ~/.loongclaw/config.toml
```

## External Skills Runtime Guardrails

External skills runtime is now safety-first by default and explicitly opt-in:

- `external_skills.enabled = false` by default (downloads/runtime disabled).
- `external_skills.require_download_approval = true` by default.
- Domain blocklist has priority over every other rule.
- If `allowed_domains` is non-empty, only allowlisted domains can be downloaded.
- `external_skills.fetch` blocks redirects to avoid silent cross-domain hops.

Recommended config baseline:

```toml
[external_skills]
enabled = true
require_download_approval = true
allowed_domains = ["skills.sh", "clawhub.io"]
blocked_domains = ["*.evil.example"]
auto_expose_installed = true
```

Agent-facing tools:

- `external_skills_policy`
  - `action=get` reads effective runtime policy.
  - `action=set` updates enable/approval/domain policy at runtime (requires `policy_update_approved=true`).
  - `action=reset` clears runtime overrides back to config defaults (requires `policy_update_approved=true`).
- `external_skills_fetch`
  - Requires `url`.
  - Requires `approval_granted=true` when approval guard is enabled.
  - Saves artifact under `<tools.file_root>/external-skills-downloads/`.
  - Enforces allowlist/blocklist before network download.
- `external_skills_install`
  - Requires local `path`.
  - Accepts a directory containing `SKILL.md` or a local `.tgz` / `.tar.gz` archive.
  - Installs the skill under `<tools.file_root>/external-skills-installed/` by default.
- `external_skills_list`
  - Lists resolved external skills across `managed`, `user`, and `project` scopes.
  - Reports lower-priority duplicates under `shadowed_skills` so operators can debug precedence.
- `external_skills_inspect`
  - Returns metadata and a short preview for the resolved skill plus any shadowed duplicates with the same `skill_id`.
- `external_skills_invoke`
  - Loads the resolved skill's `SKILL.md` instructions into the conversation loop.
- `external_skills_remove`
  - Removes a managed installed skill and updates the local index.

Operator-facing CLI:

- `loongclaw skills list [--config PATH] [--json]`
  - Lists resolved external skills across `managed`, `user`, and `project` scopes using the current config/runtime guardrails.
  - Includes `shadowed skills` in text output and `shadowed_skills` in JSON output when duplicate ids are hidden by precedence.
- `loongclaw skills info <skill-id> [--config PATH] [--json]`
  - Shows structured metadata plus a short `SKILL.md` preview for one resolved skill.
  - Includes any lower-priority duplicates that were shadowed by the selected skill.
- `loongclaw skills install <path> [--skill-id ID] [--replace] [--config PATH] [--json]`
  - Installs a local skill directory or `.tgz` / `.tar.gz` archive through the same managed runtime path as `external_skills.install`.
- `loongclaw skills remove <skill-id> [--config PATH] [--json]`
  - Removes one managed installed skill from the local index.
- `loongclaw skills policy get|set|reset [--config PATH] [--json]`
  - Reads or updates the config-backed external-skills runtime policy with the same policy fields exposed by `external_skills.policy`.
  - Mutating `set` and `reset` calls require `--approve-policy-update`.

Recommended runtime flow:

1. Download with `external_skills.fetch`
2. Install with `external_skills.install` or `loongclaw skills install`
3. Discover with `external_skills.list` or `loongclaw skills list`
   - Resolution order is `managed > user > project`
   - If a config path is loaded, project discovery anchors to that config directory; otherwise it falls back to the configured tool file root or the current working directory
   - Within the active project root, project discovery probes the current working directory and each ancestor up to the project root
   - Project discovery probes `.agents/skills`, `.codex/skills`, `.claude/skills`, and `skills/`
   - User discovery probes `~/.agents/skills`, `~/.codex/skills`, and `~/.claude/skills`
   - User and project discovery follow directory symlinks inside those skill roots; managed installs still reject symlinked sources
4. Inspect with `external_skills.inspect` or `loongclaw skills info`
5. Load instructions with `external_skills.invoke`

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
- `onboard` -- guided first-run with preflight diagnostics
- `ask` -- one-shot assistant answer and exit
- `doctor` -- diagnostics with optional safe fixes (`--fix`) and machine-readable output (`--json`)
- `chat` -- interactive CLI with sliding-window conversation memory
- Default-visible tools: `browser.open`, `browser.extract`, `browser.click`, `web.fetch`, `shell.exec`, `file.read`, `file.write`, `external_skills_policy`
- External-skills lifecycle tools (`external_skills.fetch/install/list/inspect/invoke/remove`) are advertised only when the external-skills runtime is enabled
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
| `tool-browser` | `browser.open` / `browser.extract` / `browser.click` tools |
| `tool-shell` | `shell.exec` tool |
| `tool-file` | `file.read` / `file.write` tools |
| `tool-webfetch` | `web.fetch` tool |
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
| [Roadmap](docs/ROADMAP.md) | Stage-based milestones and acceptance criteria |
| [Product Sense](docs/PRODUCT_SENSE.md) | Current MVP journey and user-facing product principles |
| [Reliability](docs/RELIABILITY.md) | Build and kernel invariants |
| [Examples](examples/README.md) | Spec files, plugin samples, benchmarks |
| [Product Specs](docs/product-specs/index.md) | User-facing requirements for onboarding, ask, doctor, channels, and WebChat expectations |
| [Skills](skills/) | Agent skills (`update-harness.skill`) |
| [Changelog](CHANGELOG.md) | Release history |

## Configuration

`loongclaw onboard` defaults to referencing provider credentials through `provider.api_key`, so secrets stay outside the config file:

```toml
[provider]
kind = "openai"
api_key = "${PROVIDER_API_KEY}"    # preferred explicit env reference
```

`provider.api_key` also accepts `$PROVIDER_API_KEY`, `env:PROVIDER_API_KEY`, `%PROVIDER_API_KEY%`, or a direct literal like `api_key = "sk-..."`.
Legacy `api_key_env = "PROVIDER_API_KEY"` remains supported for compatibility, but new configs should prefer `provider.api_key`.

Volcengine Coding Plan / ARK demo:

```toml
[provider]
kind = "volcengine"
model = "your-coding-plan-model-id"
api_key = "${ARK_API_KEY}"
base_url = "https://ark.cn-beijing.volces.com"
chat_completions_path = "/api/v3/chat/completions"
```

`kind = "volcengine"` already applies the Volcengine defaults above, so `base_url` and `chat_completions_path` are only needed when you want the config to spell them out explicitly.

### Tool policy

Shell execution defaults to **deny-unknown** — only explicitly allowed commands run.
File access is sandboxed to the working directory by default. `web.fetch` and the bounded browser
tools reuse the same SSRF-safe network policy and still block localhost, private hosts, and
special-use destinations unless the operator explicitly relaxes that policy. Runtime tool
advertising follows the active config, so disabled browser/web/external-skills surfaces disappear
from the exposed assistant tool catalog instead of drifting from reality.

```toml
[tools]
shell_default_mode = "deny"                          # "deny" | "allow"
shell_allow = ["echo", "ls", "git", "cargo"]         # permitted commands
shell_deny = []                                      # hard-blocked commands
# file_root = "/home/user/project"                   # defaults to CWD

[tools.browser]
enabled = true
max_sessions = 8
max_links = 40
max_text_chars = 6000

[tools.web]
enabled = true
allowed_domains = ["docs.example.com"]
blocked_domains = ["*.internal.example"]
max_bytes = 1048576
timeout_seconds = 15
max_redirects = 3
```

See [Tool Policy Configuration](docs/configuration/tool-policy.md) for the full reference.

Provider model-catalog cache tuning:

```toml
[provider]
model = "auto"
# Fresh cache window for /v1/models (default: 30000, max: 300000; set 0 to disable cache)
model_catalog_cache_ttl_ms = 30000
# Extra stale window used only when model-list fetch fails (default: 120000, max: 600000)
model_catalog_stale_if_error_ms = 120000
# Cache entry capacity for model catalogs (default: 32, range: 1-256)
model_catalog_cache_max_entries = 32
# Base cooldown window for model candidates rejected as incompatible (default: 300000, max: 3600000; set 0 to disable)
model_candidate_cooldown_ms = 300000
# Exponential backoff cap for repeated candidate failures (default: 3600000, max: 86400000)
model_candidate_cooldown_max_ms = 3600000
# Cache entry capacity for model candidate cooldown state (default: 64, range: 1-512)
model_candidate_cooldown_max_entries = 64
# Base cooldown for auth profiles after transient failures (default: 60000, max: 3600000; set 0 to disable)
profile_cooldown_ms = 60000
# Max cooldown cap for repeated profile failures (default: 3600000, max: 86400000)
profile_cooldown_max_ms = 3600000
# Disable window for auth-rejected profiles (default: 21600000, range: 60000-604800000)
profile_auth_reject_disable_ms = 21600000
# In-memory profile-state capacity per runtime namespace (default: 256, range: 1-1024)
profile_state_max_entries = 256
# Profile-state persistence backend ("file" or "sqlite", default: "file")
profile_state_backend = "file"
# Profile health enforcement mode ("provider_default", "enforce", "observe_only"; default: "provider_default")
# provider_default currently maps openrouter -> observe_only, others -> enforce
profile_health_mode = "provider_default"
# Optional sqlite file path when backend = "sqlite" (defaults to ~/.loongclaw/provider-profile-state.sqlite3)
profile_state_sqlite_path = "~/.loongclaw/provider-profile-state.sqlite3"
```

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
