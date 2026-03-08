# LoongClaw Roadmap

Last updated: 2026-03-08

This roadmap is execution-focused. Every stage has:

- concrete deliverables
- explicit security and stability gates
- test and CI acceptance criteria

## North Star

Build a layered Agentic OS kernel that is:

- minimal at the core
- strong at policy and safety boundaries
- deeply integrable in both directions (others integrate LoongClaw, LoongClaw integrates others)
- hot-pluggable and community-extensible without core mutation
- customizable into vertical domain systems through declarative packs

## Architecture Invariants

1. Core contracts stay small and stable.
2. Rich behavior lands in extension planes, plugin packs, and connectors.
3. High-risk actions require human authorization under policy.
4. Untrusted plugins default to strict scan + restricted runtime.
5. Every security-critical decision must be auditable.

## Stage 0: Kernel Contract Freeze (Done)

Status: complete  
Focus: create minimal but strong core boundaries.

Delivered:

- core/extension split for `runtime`, `tool`, `memory`, `connector`
- capability-based policy engine and policy extension chain
- deterministic audit timeline and event sink abstraction
- vertical pack capability/connectors boundary model

Exit criteria met:

- core boundaries enforced by unit tests
- deterministic audit schema tests passing
- property tests for capability boundary invariants passing

## Stage 1: Baseline Security & Governance (In Progress)

Status: in progress  
Focus: medium-balanced defaults + hard stops for high-risk behavior.

Delivered:

- medium-balanced human approval model with:
  - per-call allow mode
  - one-time full-access mode
  - denylist precedence
  - external risk profile loading
- plugin bridge support matrix and checksum integrity lock
- plugin security scan with `block_on_high`
- external profile integrity lock (`security_scan.profile_sha256`) with fail-closed behavior
- external profile signature verification (`security_scan.profile_signature`, ed25519)
- JSONL SIEM export lane (`security_scan.siem_export`) with optional fail-closed mode
- kernel-level tool-call policy gate (`PolicyEngine::check_tool_call`) with explicit
  deny/approval-required outcomes before tool dispatch (Rule of Two)
- WASM static scan controls:
  - allowed artifact paths
  - module size cap
  - digest pin support
  - import policy (`allow_wasi`, blocked prefixes)
- typed audit emission for security scan summary (`SecurityScanEvaluated`)
- per-finding correlation IDs for deterministic audit aggregation

Remaining:

- profile signing key lifecycle (rotation/revocation) and trust anchor management
- SIEM transport adapters beyond file JSONL (HTTP/syslog/event bus)

Exit criteria:

- all high-risk tool calls blocked without explicit approval
- blocked community plugin paths demonstrably fail closed
- security scan and approval decisions both visible in structured audit

## Stage 2: Safe Hotplug Runtime (Next)

Status: in progress  
Focus: runtime-grade isolation for untrusted extension execution.

Delivered in current baseline:

- WASM runtime execution lane wired into `bridge_execution` with Wasmtime backend.
- Policy-driven runtime guardrails in `bridge_support.security_scan.runtime`:
  - required `allowed_path_prefixes` when `execute_wasm_component=true` (fail closed)
  - `max_component_bytes`
  - optional `fuel_limit`
- Runtime isolation tests for:
  - successful wasm execution
  - runtime prefix denial
  - runtime size-limit denial
  - invalid runtime policy denial

Remaining deliverables:

- WASM runtime lane with enforced resource limits:
  - CPU budget refinement
  - memory limits
  - timeout/termination policy
- process bridge sandbox profile tiers (`restricted`, `balanced`, `trusted`)
- hot-reload lifecycle hooks:
  - pre-load validation
  - rollback-on-failure
  - post-load health check

Acceptance criteria:

- runtime isolation e2e tests (normal + adversarial cases) passing
- deterministic rollback behavior validated under injected failures
- no core-path mutation allowed by plugin hotplug workflow

## Stage 3: Autonomous Integration Expansion (In Progress)

Status: in progress  
Focus: dynamic provider/channel integration without hardcoding.

Delivered in current baseline:

- protocol foundation crate (`crates/protocol`) with:
  - transport contract (`Transport` trait + typed frame envelopes)
  - typed method routing (`ProtocolRoute`) and policy-aware resolver (`ProtocolRouter`)
  - strict/lenient custom route control to avoid ad-hoc string dispatch at call sites
  - linked in-memory `ChannelTransport` primitive with:
    - bounded queue backpressure
    - explicit close semantics
    - deterministic async transport tests (roundtrip, close behavior, backpressure)
- `tool_search` operation for runtime tool discovery over:
  - loaded providers in integration catalog
  - scanned-but-not-absorbed plugin descriptors
- translation-aligned retrieval payloads:
  - runtime profile hints (`bridge_kind`, `adapter_family`, `entrypoint_hint`, `source_language`)
  - plugin semantic fields (`summary`, `tags`, `input_examples`, `output_examples`, `defer_loading`)
- `programmatic_tool_call` operation for server-side tool orchestration:
  - step model (`set_literal`, `json_pointer`, `connector_call`, `connector_batch`, `conditional`)
  - connector allowlist and call-budget enforcement
  - batch execution controls (`parallel`, `continue_on_error`) with per-call structured outcomes
  - branch predicates (`equals`, `exists`) for deterministic conditional routing
  - per-call retry/backoff policy (`max_attempts`, `initial_backoff_ms`, `max_backoff_ms`)
  - deterministic adaptive retry jitter (`jitter_ratio`, `adaptive_jitter`)
  - per-connector rate shaping policy (`connector_rate_limits.<connector>.min_interval_ms`)
  - per-connector circuit breaker policy (`failure_threshold`, `cooldown_ms`,
    `half_open_max_calls`, `success_threshold`)
  - adaptive concurrency policy (`concurrency`) with:
    - global in-flight cap (`max_in_flight`)
    - explicit floor and ramp profile (`min_in_flight`, adaptive up/down steps)
    - fair scheduling policy (`weighted_round_robin` / `strict_round_robin`)
    - per-call priority classes (`high` / `normal` / `low`)
    - policy-driven adaptive budget contraction/recovery triggers (`adaptive_reduce_on`)
  - scheduler telemetry for fanout (`dispatch_order`, `peak_in_flight`,
    `budget_reductions`, `budget_increases`, `final_in_flight_budget`)
  - typed programmatic error taxonomy (`programmatic_error[code]`, batch `error_code`)
  - return-step targeting and optional intermediate traces
  - payload templating (`{{step_id}}`, `{{step_id#/json/pointer}}`)
- dynamic connector caller ACL:
  - `allowed_callers` and `allowed_callers_json` metadata gates
  - automatic `_loongclaw.caller` provenance injection for programmatic calls
- active `http_json` runtime execution lane (no longer plan-only):
  - timeout-controlled request execution
  - structured runtime evidence (`status_code`, `response_json`)

Planned deliverables:

- connector contract versioning and compatibility matrix
- provider/channel auto-provision enhancements:
  - missing connector discovery strategy
  - deterministic configuration synthesis
  - idempotent reconciliation
- bi-directional protocol bridge adapters:
  - OpenAI-compatible
  - Anthropic-compatible
  - MCP server/client lanes

Acceptance criteria:

- repeated auto-provision runs are idempotent
- unsupported protocol paths fail with explicit typed reasons
- full integration catalog diff is auditable and reversible
- discovery + orchestration tests remain stable under mixed absorbed/deferred plugin states

## Stage 4: Community Plugin Supply Chain (Next)

Status: planned  
Focus: open ecosystem without sacrificing trust boundaries.

Planned deliverables:

- multi-language plugin intake pipeline:
  - manifest extraction
  - bridge inference
  - safety classification
- plugin packaging and signing metadata
- trust policy tiers (`official`, `verified-community`, `unverified`)
- reproducible plugin artifact verification in CI

Acceptance criteria:

- unsigned/untrusted high-risk plugins never auto-activate
- plugin provenance visible in catalog and audit events
- plugin translation + activation plans deterministic across runs

## Stage 5: Vertical Pack Productization (Next)

Status: planned  
Focus: 15-minute vertical customization workflow.

Planned deliverables:

- pack template generator:
  - domain prompt baseline
  - tool/connector policy presets
  - evaluation set bootstrap
- pack-level SLO/quality dashboard:
  - latency
  - success ratio
  - safety violations
- guided hardening checklist per vertical pack

Acceptance criteria:

- new vertical pack reaches runnable state in <= 15 minutes
- pack policy and required capabilities fully declarative
- regression pack tests can be generated and executed automatically

## Stage M: End-User MVP Product Layer (In Progress)

Status: in progress  
Focus: ship a low-friction daily-usable daemon entry for non-developers.

Delivered in current baseline:

- `setup` command to generate TOML configuration and bootstrap local state
- `chat` command as baseline CLI channel
- first-party Telegram polling channel adapter
- first-party Feishu webhook channel adapter
- SQLite-backed conversation memory with sliding-window retrieval
- core tool execution for `shell.exec`, `file.read`, `file.write`
- one-command source install scripts (`scripts/install.sh`, `scripts/install.ps1`)
- Cargo feature flags for MVP packaging controls
- modular channel/provider architecture for extension-safe evolution:
  - `mvp/channel/feishu/*` split into adapter/payload/webhook layers
  - Feishu encrypted webhook payload decrypt lane with signature verification
  - `mvp/provider/*` split into policy/transport/shape layers
  - `ConversationRuntime` port for non-invasive backend extension and contract testing
- daemon runtime entrypoint decomposition:
  - `crates/daemon/src/main.rs` reduced to CLI routing + bootstrap wiring
  - spec/runtime models and adapter inventory extracted to `spec_runtime.inc.rs`
  - heavy spec execution/security/approval pipeline extracted to `spec_execution.inc.rs`
  - keeps behavior stable while removing multi-thousand-line single-file coupling

Remaining deliverables:

- OpenAI-compatible protocol adapter hardening and Volcengine custom adapter profile
- beginner installation pipeline:
  - prebuilt binaries
  - one-command setup on macOS/Linux/Windows
  - guided onboarding flow and diagnostics

Acceptance criteria:

- a new user can install and complete first successful chat in <= 5 minutes
- local memory persistence is stable across process restarts
- shell/file tools obey policy constraints and emit auditable outcomes
- channel/provider modules can be toggled by feature flags without core code edits

## Quality Gate Matrix (Always On)

All roadmap stages must keep these gates green:

1. `cargo fmt`
2. `cargo test` (workspace full pass)
3. Security regression set (approval, scan, bridge constraints)
4. Audit schema stability checks for critical event kinds
5. No hardcoded risk exceptions when config-driven alternatives exist

## Current Priority Order

1. Stage 2 hardening: memory/timeout isolation + sandbox tiers
2. Stage 2 hot-reload reliability: pre/post checks + rollback contract
3. Stage 3 baseline: connector contract versioning and idempotent reconciliation
