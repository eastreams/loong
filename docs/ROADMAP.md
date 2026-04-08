# LoongClaw Roadmap

Last updated: 2026-03-20

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
- process bridge sandbox profile tiers (`restricted`, `balanced`, `trusted`) aligned with the
  shared execution-tier contract used by browser and WASM evidence surfaces
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
  - route authorization contract (`RouteAuthorizationRequest`) for deterministic
    auth/capability gates before handler dispatch
  - json-line stream transport (`JsonLineTransport`) for stdio/pipe integration
    with deterministic decode/error handling and close semantics
  - daemon `process_stdio` bridge now executes through protocol json-line frames
    (request/response method + payload envelope), with runtime evidence
    (`transport_kind`, request/response frame metadata) and deterministic
    failure surfacing for malformed frame responses
  - daemon `process_stdio` bridge hardening:
    - protocol route authorization gate before process execution
    - response contract checks (method/id must match request)
    - bounded send/close/recv/exit timeouts (`process_timeout_ms`)
  - daemon `http_json` bridge hardening:
    - protocol route authorization gate before HTTP request
    - protocol runtime evidence (`request_method/id`, required capability)
    - optional strict response contract mode (`http_enforce_protocol_contract`)
      validating response `method` + `id`
    - bounded HTTP timeout parsing with deterministic clamp (`http_timeout_ms`)
  - shared protocol context builder for bridge executors to keep
    authorization/route semantics consistent across transport lanes
  - shared runtime evidence appender for protocol request/route/capability
    fields to keep bridge telemetry schemas aligned across executors
  - bridge protocol helpers split into a dedicated module include
    (`spec_bridge_protocol.inc.rs`) to reduce spec runtime file growth
  - bridge-focused spec runtime tests split into dedicated module
    (`tests/spec_runtime_bridge.rs`) to reduce test-file maintenance debt
  - typed bridge runtime evidence structs with shared serialization path to
    reduce ad-hoc JSON field drift across executors
  - explicit runtime evidence state variants (`BaseOnly`/`RequestOnly`/
    `Response` or `Execution`) to avoid impossible field combinations
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
- builtin-only memory-system foundation for `dev`:
  - typed memory-system metadata and registry seam
  - hydrated memory orchestration over LoongClaw-owned canonical history
  - operator diagnostics for selected system, capability set, and effective
    memory fail-open policy

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
  - manifest-first package discovery with file contract precedence over
    embedded source markers
  - bridge inference
  - safety classification
- setup-only plugin metadata and governed setup-entry contract for onboarding,
  install, and doctor
- slot-aware ownership model for exclusive vs shared plugin-provided runtime
  surfaces
- plugin packaging and signing metadata
- trust policy tiers (`official`, `verified-community`, `unverified`)
- reproducible plugin artifact verification in CI

Acceptance criteria:

- unsigned/untrusted high-risk plugins never auto-activate
- plugin provenance visible in catalog and audit events
- plugin setup guidance can render from manifest metadata without executing the
  plugin runtime
- exclusive/shared ownership conflicts are deterministic and auditable
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

- `onboard` command as the primary first-run configuration and diagnostics flow
- `ask` command as the one-shot assistant fast path
- `chat` command as baseline CLI channel
- `doctor` repair loop with `--fix` and machine-readable output
- ask-first first-run handoff from onboarding and doctor with concrete next-step guidance
- first-party Telegram polling channel adapter
- first-party Feishu webhook channel adapter
- SQLite-backed conversation memory with sliding-window retrieval
- core tool execution for `browser.open`, `browser.extract`, `browser.click`, `web.fetch`, `shell.exec`, `file.read`, `file.write`, `file.edit`
- release-first install flow with checksum-verified prebuilt binaries and explicit source fallback (`scripts/install.sh`, `scripts/install.ps1`)
- runtime-visible tool advertising so capability snapshots and provider tool schemas follow the actually enabled tool surface
- Cargo feature flags for MVP packaging controls
- product specs for installation, onboarding, one-shot ask, doctor, browser automation, tool surface, channel setup, runtime experiment, and Web UI expectations
- experiment-state operator surface foundation:
  - `runtime-snapshot` persists lineage-aware runtime checkpoint artifacts
  - `runtime-restore` replays a persisted checkpoint as a dry-run or apply plan
  - `runtime-experiment start|finish|show|compare` records baseline snapshot, mutation summary, result snapshot, evaluation metrics, warnings, final decision, and optional snapshot-backed runtime deltas for operator review
  - `runtime-capability propose|review|show` records one run-derived capability candidate, bounded scope, required capabilities, explicit operator review, and any recorded snapshot-backed delta evidence without mutating live runtime state
  - `runtime-capability index` groups matching candidate records into deterministic capability families, emits compact evidence digests including delta-evidence coverage and changed runtime surfaces, and evaluates readiness as `ready`, `not_ready`, or `blocked`
  - `runtime-capability plan` resolves one indexed capability family into a deterministic dry-run promotion plan with artifact identity, blockers, approval checklist, rollback hints, provenance, family-level delta evidence digest, and a structured draft payload preview
  - `runtime-capability apply` materializes one governed draft artifact under the planned delivery surface for a promotable family, reuses the planned payload shape, and keeps repeated applies idempotent while leaving live runtime state untouched
- modular channel/provider architecture for extension-safe evolution:
  - `app/channel/feishu/*` split into adapter/payload/webhook layers
  - Feishu encrypted webhook payload decrypt lane with signature verification
  - `app/provider/*` split into policy/transport/shape layers
  - `ConversationRuntime` port for non-invasive backend extension and contract testing
- daemon runtime entrypoint decomposition:
  - `crates/daemon/src/main.rs` reduced to CLI routing + bootstrap wiring
  - spec/runtime models and adapter inventory extracted to `spec_runtime.inc.rs`
  - heavy spec execution/security/approval pipeline extracted to `spec_execution.inc.rs`
  - keeps behavior stable while removing multi-thousand-line single-file coupling

Remaining deliverables:

- OpenAI-compatible protocol adapter hardening and Volcengine custom adapter profile
- beginner installation hardening:
  - sustain tagged release publishing across macOS/Linux/Windows
  - expand beyond installer scripts into package-manager distribution only after release adoption is stable
- experiment-state operator surface follow-through:
  - use the shipped snapshot/restore/experiment/capability record layer as the prerequisite for later evaluator pipelines and automated skill-optimization loops
  - keep the new dry-run promotion planner read-only and use it as the contract for any future promotion executor instead of jumping directly to automatic mutation
- managed browser automation companion:
  - keep `browser.open`, `browser.extract`, and `browser.click` as the shipped safe browser lane
  - partial governed adapter skeleton now exists for richer page actions:
    `browser.companion.*` becomes runtime-visible only when the companion is ready, read actions stay in the Core lane, and write actions stay in the governed App lane
  - still wire install, `onboard`, and `doctor` into companion presence, version, and isolated profile health
  - still add isolated browser profile lifecycle and release packaging around the companion runtime
  - keep richer browser automation exposed only through truthful runtime-visible tool advertising and governed tool contracts
- browser-facing product surface:
  - Web UI implementation as a thin shell over existing ask/chat, onboarding, dashboard, and browser semantics, not a separate assistant runtime

Acceptance criteria:

- a new user can install and complete a first successful `ask` or `chat` in <= 5 minutes
- local memory persistence is stable across process restarts
- shell/file/web/browser tools obey policy constraints and emit auditable outcomes
- advertised tools match the actually invokable runtime surface for the current config and compiled features
- channel/provider modules can be toggled by feature flags without core code edits

## Quality Gate Matrix (Always On)

All roadmap stages must keep these gates green:

1. `cargo fmt`
2. `cargo test` (workspace full pass)
3. Security regression set (approval, scan, bridge constraints)
4. Audit schema stability checks for critical event kinds
5. No hardcoded risk exceptions when config-driven alternatives exist

## Discussion: Post-MVP Foundation Items

These items emerged from the Phase 0–3 restructure (PR #15). They are candidates for near-term work but need prioritization discussion before commitment.

### D1: Wire Phase 3 primitives into production paths

Phase 3 added generation tokens, Fault, TaskState FSM, and Namespace as additive types with tests. They are not yet used in production code paths.

Candidates:
- Issue tokens with membrane scoped to Namespace during `bootstrap_kernel_context`
- Use `TaskSupervisor` in spec runner's `execute_task` path for FSM-enforced lifecycle
- Return `Fault` from kernel dispatch methods alongside `KernelError` for caller-side recovery matching
- Use generation-based revocation for session rotation (e.g., Telegram channel restart)

Trade-off: wiring now locks in the API surface; waiting allows more usage patterns to emerge.

### D2: Persistent audit sink

`InMemoryAuditSink` loses all audit events on process restart. For security-critical decisions (policy denials, token revocations) to be auditable post-incident, a durable sink is needed.

Options:
- SQLite audit table (reuse existing rusqlite dependency)
- Append-only JSONL file (simplest, grep-friendly)
- SIEM export lane (already planned in Stage 1)

Trade-off: SQLite is queryable but adds schema migration burden. JSONL is zero-schema but harder to query.

### D3: Make `persist_turn` async

`persist_turn` currently uses `tokio::task::block_in_place` to bridge sync → async kernel calls. This works but blocks a tokio worker thread and panics on single-threaded runtimes.

Approach: make `ConversationRuntime::persist_turn` async, update `ConversationOrchestrator` callers. Straightforward but touches 6+ files.

Trade-off: low risk, moderate churn. Unblocks single-threaded runtime compatibility.

### D4: Route `build_messages` memory window through kernel

`build_messages_for_session` in the MVP provider layer couples system prompt construction with SQLite memory window loading. The memory-read portion should route through the kernel's memory plane for policy/audit coverage.

Approach: split `build_messages_for_session` into prompt construction + kernel-routed memory read. Requires new `MemoryCoreRequest` operation (e.g., `read_window`).

Trade-off: improves audit coverage for memory reads, but requires splitting a tightly coupled function.

Status (`dev`): implemented.
- Added `ConversationContextEngine` seam and default implementation.
- `build_messages` now assembles through context engine and routes memory window reads via `kernel.execute_memory_core(..., Capability::MemoryRead, ...)` when kernel context is present.
- Added registry/selection hooks (`register_context_engine`, `resolve_context_engine`, `LOONGCLAW_CONTEXT_ENGINE`) plus config-based selector (`[conversation].context_engine`) for future multi-engine evolution without invasive runtime refactors.
- Added built-in `legacy` engine for rollback and behavior comparison against pre-seam assembly path.
- Added engine metadata surface (`id`, `api_version`, `capabilities`) for diagnostics and compatibility checks before introducing more advanced engines.
- Added explicit post-turn context compaction hook (`compact_context`) to reserve an upgrade seam for summarization/compression without rewriting orchestrator flow.
- Added reserved context-engine lifecycle hooks (`bootstrap`, `ingest`) as default no-op seams so future engine-owned import/indexing flows do not require another trait/runtime rewrite.
- Added reserved subagent lifecycle hooks (`prepare_subagent_spawn`, `on_subagent_ended`) as default no-op seams so future multi-agent context wiring avoids trait-breaking refactors.
- Added richer context assembly output (`messages`, optional `estimated_tokens`, optional `system_prompt_addition`) to pre-wire policy/runtime prompt augmentation without revisiting runtime boundaries.
- Added compaction policy controls (`compact_enabled`, `compact_min_messages`, `compact_trigger_estimated_tokens`, `compact_fail_open`) so heavy future summarization can be enabled/tuned without orchestrator rewrites.
- Added daemon observability command (`list-context-engines`) to show selected engine source (env/config/default) and available engine capabilities.
- Added unified runtime snapshot assembly for context engine diagnostics (selected + available + compaction policy) to keep CLI/channel observability outputs consistent.
- Added regression tests for kernel-routed window reads, runtime injection, and registry resolution.

### D5: Upgrade `InMemoryAuditSink` to queryable snapshot

Current `InMemoryAuditSink::snapshot()` returns a full clone of all events. For long-running processes, this grows unboundedly.

Options:
- Add ring-buffer with configurable capacity
- Add filtered snapshot (by time range, event kind, pack_id)
- Combine with D2 (persistent sink replaces in-memory for production)

Trade-off: if D2 lands, this becomes test-only infrastructure. May not be worth optimizing independently.

### D6: Retire governed/direct runtime drift

`ConversationRuntimeBinding` and `ProviderRuntimeBinding` make governance explicit, but `Direct`
still survives as a compatibility lane deeper in the runtime than the long-term architecture wants.
The next kernel-first closure track should push direct behavior back toward ingress, compatibility
wrappers, and tests, while keeping governed reads and governed side effects fail-closed where
possible.

Trade-off: improves architecture truthfulness and future maintainability, but must be executed in
small bounded slices instead of one repo-wide kernelization patch.

### D7: ACP control-plane hardening and recovery

ACP is now a real control plane rather than an experiment, but too much lifecycle, observability,
and backend transport behavior still concentrates in a few large hotspots. The next ACP work should
focus on decomposition, stuck-turn recovery, cancel/close repair, and clearer observability before
adding more surface breadth.

Trade-off: lowers merge risk and control-plane debt, but requires disciplined ownership extraction
instead of feature-driven growth inside large files.

### D8: Shared execution security tiers

The roadmap already names process sandbox profile tiers, but the wider runtime still needs one
shared execution-tier vocabulary across process, browser, and WASM lanes. Without that, each lane
risks growing its own security semantics and evidence model.

Trade-off: the first slice should standardize the contract, not attempt a giant all-lane sandbox
rewrite.

Current first-slice mapping:

- `restricted` - built-in browser lane and the current WASM component runtime lane
- `balanced` - allowlisted `process_stdio` bridge execution and the managed browser companion when
  its runtime gate is open
- `trusted` - reserved for future explicit high-trust runtime lanes rather than assumed by default

### D9: First-party workflow packs on hardened primitives

Once the runtime base is harder, LoongClaw should turn that into a small set of first-party
workflow packs that prove the kernel's value in operator-facing tasks such as release/review work,
issue triage, or channel support.

Trade-off: this is the right productization direction, but it should follow runtime hardening
instead of preceding it.

## Current Priority Order

1. Kernel-first runtime closure and direct-path retirement
2. Persistent audit sink and query baseline
3. ACP control-plane hardening and recovery
4. Shared execution security tiers across process/browser/WASM lanes
5. First-party workflow packs on hardened runtime primitives

Execution package for this order:

- the 2026-03-17 internal runtime hardening design in `docs/plans/`
- the 2026-03-17 internal runtime hardening implementation plan in `docs/plans/`
- the 2026-03-17 internal runtime hardening GitHub backlog in `docs/plans/`
