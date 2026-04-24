# Loong Roadmap

Last updated: 2026-03-29

The reader-facing summary for this material lives in
[`../site/reference/roadmap-and-product.mdx`](../site/reference/roadmap-and-product.mdx).
This file remains the repository-native roadmap source for maintainers and
source readers.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read the public roadmap and product summary first | [`../site/reference/roadmap-and-product.mdx`](../site/reference/roadmap-and-product.mdx) |
| inspect the full repository-native roadmap source | this file |
| understand the broader repository docs layering | [`README.md`](README.md) |

## Read This File When

- you need the full repository-native execution roadmap instead of the public summary
- you are reviewing stage exit criteria, not just public positioning
- you are checking whether a roadmap item is already delivered, in progress, or still next

## How To Read This File

This roadmap is execution-focused. Every stage has:

- concrete deliverables
- explicit security and stability gates
- test and CI acceptance criteria

## Current Stage Summary

| Stage | Status | Use the detailed section when... |
| --- | --- | --- |
| Stage 0: Kernel Contract Freeze | Done | you need the original core-boundary baseline |
| Stage 1: Baseline Security & Governance | In Progress | you are working on policy, approval, scan, or audit hardening |
| Stage 2: Safe Hotplug Runtime | Next | you are planning runtime isolation or hotplug hardening work |
| Stage 3: Autonomous Integration Expansion | In Progress | you are working on protocol, provider, channel, or discovery expansion |
| Stage 4: Community Plugin Supply Chain | Next | you are working on package intake, trust, or plugin verification |
| Stage 5: Vertical Pack Productization | Next | you are shaping reusable vertical-pack authoring and hardening flows |
| Stage M: End-User MVP Product Layer | In Progress | you are touching the first-run product surface, local operator UX, or gateway-owned delivery |

The discussion section at the end stays intentionally narrower than the internal
backlog. It only keeps cross-cutting public follow-up items that still matter to
source readers.

## North Star

Build a layered Agentic OS kernel that is:

- minimal at the core
- strong at policy and safety boundaries
- deeply integrable in both directions (others integrate Loong, Loong integrates others)
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
- kernel-level request-policy gate for tool calls through `PolicyEngine::authorize(...)`
  plus `PolicyExtensionChain`, with explicit deny/approval-required outcomes before
  tool dispatch (Rule of Two)
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

Status: next
Focus: runtime-grade isolation for untrusted extension execution.

Current baseline already in place:

- WASM runtime execution lane wired into `bridge_execution` with Wasmtime backend.
- Core-module WASM host ABI v0 for plugin data exchange:
  - request payload delivery into guest memory
  - structured JSON output capture from guest memory
  - allowlisted guest-readable config access via namespaced `provider.` / `channel.` keys
  - bounded guest logging surfaced in runtime evidence
  - explicit guest abort propagation
  - backward-compatible fallback to legacy `run() -> ()`
- Policy-driven runtime guardrails in `bridge_support.security_scan.runtime`:
  - required `allowed_path_prefixes` when `execute_wasm_component=true` (fail closed)
  - optional `guest_readable_config_keys` allowlist for WASM guest config reads
  - `max_component_bytes`
  - optional `max_output_bytes` for host ABI output capture
  - optional `fuel_limit`
  - optional `timeout_ms` enforced through Wasmtime epoch interruption
- Runtime isolation tests for:
  - successful wasm execution
  - timeout-guarded execution without cache reuse
  - timeout-triggered termination for non-returning modules
  - runtime prefix denial
  - runtime size-limit denial
  - invalid runtime policy denial

Remaining deliverables:

- WASM runtime lane with enforced resource limits:
  - CPU budget refinement
  - memory limits
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

Current baseline already in place:

### Protocol Foundation And Bridge Contract

- protocol foundation crate (`crates/protocol`) with:
  - transport contract (`Transport` trait + typed frame envelopes)
  - typed method routing (`ProtocolRoute`) and policy-aware resolver (`ProtocolRouter`)
  - route authorization contract (`RouteAuthorizationRequest`) for deterministic
    auth/capability gates before handler dispatch
  - json-line stream transport (`JsonLineTransport`) for stdio/pipe integration
    with deterministic decode/error handling and close semantics
- daemon bridge hardening and shared helpers:
  - `process_stdio` now executes through protocol json-line frames with runtime
    evidence (`transport_kind`, request/response frame metadata) and
    deterministic malformed-response surfacing
  - `process_stdio` route authorization, response contract checks, and bounded
    send/close/recv/exit timeouts (`process_timeout_ms`)
  - `http_json` route authorization, protocol runtime evidence, optional
    strict response contract mode (`http_enforce_protocol_contract`), and
    bounded timeout parsing (`http_timeout_ms`)
  - shared protocol context builder and shared runtime evidence appender for
    consistent bridge semantics across transport lanes
  - dedicated bridge helper and test modules
    (`spec_bridge_protocol.inc.rs`, `tests/spec_runtime_bridge.rs`) to reduce
    runtime and spec maintenance drift
  - typed bridge runtime evidence structs and explicit evidence state variants
    (`BaseOnly` / `RequestOnly` / `Response` / `Execution`) to avoid impossible
    field combinations
  - strict/lenient custom route control to avoid ad-hoc string dispatch
- linked in-memory `ChannelTransport` primitive with bounded queue backpressure,
  explicit close semantics, and deterministic async tests

### Tool Discovery And Retained Audit Review

- `tool_search` operation for runtime tool discovery over loaded providers and
  scanned-but-not-absorbed plugin descriptors
- trust-aware discovery controls and summaries:
  - query prefixes such as `trust:official` and `tier:verified-community`
  - structured `trust_tiers` fields
  - operator-visible `trust_filter_summary`
  - top-level `tool_search_summary`
  - `run-spec --render-summary` stderr rendering that preserves stdout JSON consumers
- typed discovery audit emission (`ToolSearchEvaluated`) plus retained-review aids:
  - summary hints (`last_triage_label`, `last_triage_summary`, `last_triage_hint`)
  - audit filters on kind, triage label, query substring, requested/effective
    trust tier, pack, agent, event, and token
  - dedicated `audit discovery` and `audit token-trail` operator views
  - inclusive time-window filters across recent/summary/discovery
  - grouped rollups for `audit summary --group-by pack|agent|token`
  - grouped rollups for `audit discovery --group-by pack|agent`
  - grouped discovery drill-down, correlated summary, remediation hint, and
    remediation command handoff so hotspots stay actionable

### Retrieval Payload Alignment

- translation-aligned retrieval payloads:
  - runtime profile hints (`bridge_kind`, `adapter_family`, `entrypoint_hint`, `source_language`)
  - plugin semantic fields (`summary`, `tags`, `input_examples`, `output_examples`, `defer_loading`)
  - plugin provenance and trust fields (`provenance_summary`, `trust_tier`)

### Programmatic Orchestration Surface

- `programmatic_tool_call` operation for server-side tool orchestration:
  - step model (`set_literal`, `json_pointer`, `connector_call`, `connector_batch`, `conditional`)
  - connector allowlist and call-budget enforcement
  - batch execution controls (`parallel`, `continue_on_error`) with per-call structured outcomes
  - branch predicates (`equals`, `exists`) for deterministic conditional routing
  - retry/backoff policy plus deterministic adaptive jitter
  - per-connector rate shaping and circuit-breaker policy
  - adaptive concurrency policy with global cap, explicit floor/ramp profile,
    fair scheduling, priority classes, and policy-driven budget contraction or recovery
  - scheduler telemetry (`dispatch_order`, `peak_in_flight`, `budget_reductions`,
    `budget_increases`, `final_in_flight_budget`)
  - typed programmatic error taxonomy, return-step targeting, optional
    intermediate traces, and payload templating
- dynamic connector caller ACL:
  - `allowed_callers` and `allowed_callers_json` metadata gates
  - automatic `_loong.caller` provenance injection for programmatic calls

### Active Runtime Lanes And Foundations

- active `http_json` runtime execution lane (no longer plan-only):
  - timeout-controlled request execution
  - structured runtime evidence (`status_code`, `response_json`)
- memory-system foundation for `dev`:
  - typed memory-system metadata and registry seam
  - hydrated memory orchestration over Loong-owned canonical history
  - result-first retrieval-plan envelope with explicit request/planner-snapshot
    compatibility adapters for legacy request-only systems
  - symmetric explicit helper pair for request-only and plan-result-only
    compatibility paths while keeping the generic trait defaults intentionally
    asymmetric until a wider policy flip is accepted
  - direct and boxed regression coverage now fence the compatibility matrix
    across request-only, plan-result-only, and shipped built-in memory systems;
    direct and boxed request/snapshot alignment is now explicit for `builtin`,
    `recall_first`, and `workspace_recall`
  - the remaining decision is whether to keep that asymmetric contract or later
    promote result-first defaults as policy
  - stable planner diagnostics carrying memory-system identity through stage
    envelopes and protocol decode/backfill paths
  - operator diagnostics for selected system, capability set, and effective
    memory fail-open policy

### Planned Next Deliverables

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

Current baseline already in place:

- `loong plugins init <package_root>` scaffolds a manifest-first plugin
  package root with a canonical `loong.plugin.json`, current host
  compatibility defaults, and a README that routes authors into shared
  package diagnosis instead of internal crate spelunking
- `loong plugins doctor --root <package_root>` reuses the shared
  `plugin_preflight` contract for author-facing package diagnosis, defaulting
  to the `sdk_release` profile while surfacing setup truth, remediation
  classes, and required operator follow-up actions
- package-manifest runtime projection now also honors explicit
  `metadata.source_language`, so language-specific scaffolded packages keep
  canonical bridge, adapter-family, and preflight language semantics

### Package Intake Direction

- multi-language plugin intake pipeline:
  - manifest-first package discovery with file contract precedence over
    embedded source markers
  - bridge inference
  - safety classification
- setup-only plugin metadata and governed setup-entry contract for onboarding,
  install, and doctor
- slot-aware ownership model for exclusive vs shared plugin-provided runtime
  surfaces

### Trust And Verification Direction

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

This stage is still outcome-first rather than package-first. It depends on the
runtime, plugin, and product-surface foundations above being explicit before
Loong turns them into reusable vertical-pack flows.

### Pack Authoring Direction

- pack template generator:
  - domain prompt baseline
  - tool/connector policy presets
  - evaluation set bootstrap

### Runtime Quality Direction

- pack-level SLO/quality dashboard:
  - latency
  - success ratio
  - safety violations

### Hardening Direction

- guided hardening checklist per vertical pack

Acceptance criteria:

- new vertical pack reaches runnable state in <= 15 minutes
- pack policy and required capabilities fully declarative
- regression pack tests can be generated and executed automatically

## Stage M: End-User MVP Product Layer (In Progress)

Status: in progress
Focus: ship a low-friction daily-usable daemon entry for non-developers.

### Core Operator Path

- `onboard` command as the primary first-run configuration and diagnostics flow
- `ask` command as the one-shot assistant fast path
- `chat` command as baseline CLI channel
- `doctor` repair loop with `--fix` and machine-readable output
- ask-first first-run handoff from onboarding and doctor with concrete next-step guidance
- release-first install flow with checksum-verified prebuilt binaries and explicit source fallback (`scripts/install.sh`, `scripts/install.ps1`)
- public product specs for installation, onboarding, one-shot ask, doctor, browser automation, tool surface, channel setup, prompt and personality, memory profiles, and shell completion

### Runtime And Delivery Baseline

- first-party Telegram polling channel adapter
- first-party Feishu webhook channel adapter
- SQLite-backed conversation memory with sliding-window retrieval
- core tool execution for `browser.open`, `browser.extract`, `browser.click`, `web.fetch`, `shell.exec`, `file.read`, `file.write`, `file.edit`
- runtime-visible tool advertising so capability snapshots and provider tool schemas follow the actually enabled tool surface
- Cargo feature flags for MVP packaging controls

### Experiment-State Foundation

- `runtime snapshot` persists lineage-aware runtime checkpoint artifacts
- `runtime restore` replays a persisted checkpoint as a dry-run or apply plan
- `runtime experiment start|finish|show|compare` records baseline snapshot, mutation summary, result snapshot, evaluation metrics, warnings, final decision, and optional snapshot-backed runtime deltas for operator review
- `runtime capability propose|review|show` records one run-derived capability candidate, bounded scope, required capabilities, explicit operator review, and any recorded snapshot-backed delta evidence without mutating live runtime state
- `runtime capability index` groups matching candidate records into deterministic capability families, emits compact evidence digests including delta-evidence coverage and changed runtime surfaces, and evaluates readiness as `ready`, `not_ready`, or `blocked`
- `runtime capability plan` resolves one indexed capability family into a deterministic dry-run promotion plan with artifact identity, blockers, approval checklist, rollback hints, provenance, and the same family-level delta evidence digest
- `runtime capability apply` materializes one deterministic governed `memory_stage_profile` artifact from a promotable capability family, keeps the output idempotent, and rejects conflicting or unsupported apply paths instead of mutating live runtime state directly

### Runtime Architecture Hardening

- modular channel/provider architecture for extension-safe evolution:
  - `app/channel/feishu/*` split into adapter/payload/webhook layers
  - Feishu encrypted webhook payload decrypt lane with signature verification
  - `app/provider/*` split into policy/transport/shape layers
  - `ConversationRuntime` port for non-invasive backend extension and contract testing
- daemon runtime entrypoint decomposition:
  - `crates/daemon/src/main.rs` reduced to CLI routing + bootstrap wiring
  - `crates/spec/src/spec_runtime.rs` now keeps runtime contracts/orchestration at the root while helper slices live under `crates/spec/src/spec_runtime/*`
  - `crates/spec/src/spec_execution.rs` now keeps execution/security/approval orchestration at the root while helper slices live under `crates/spec/src/spec_execution/*`
  - keeps behavior stable while removing multi-thousand-line single-file coupling

### Next Public Product Tracks

The roadmap keeps the public direction visible here without turning this file
into a backlog dump. Fuller implementation packages stay out of the
reader-facing docs flow until their public contracts are ready.

#### Install And Provider Hardening

- OpenAI-compatible protocol adapter hardening and Volcengine custom adapter profile
- beginner installation hardening:
  - sustain tagged release publishing across macOS/Linux/Windows
  - expand beyond installer scripts into package-manager distribution only after release adoption is stable

#### Experiment-State Follow-Through

- experiment-state operator surface follow-through:
  - use the shipped snapshot/restore/experiment/capability record layer as the prerequisite for later evaluator pipelines and automated skill-optimization loops
  - keep the new promotion planner as the contract for governed executors; only the explicit `memory_stage_profile` apply lane is shipped today, and other promotion targets stay read-only until their executor contracts exist

#### Task, Skills, And Retrieval UX

- runtime productization over already-shipped substrate:
  - background task UX on top of session runtime:
    - expose task-shaped create, inspect, wait, follow, cancel, and recover flows over the current async delegate child-session substrate
    - surface approval-pending and tool-narrowing state as task diagnostics instead of raw session-runtime detail only
    - keep cron, heartbeat, and service-owned scheduling out of the first slice
  - product-mode managed skills UX:
    - add search, recommendation, and explicit acquisition guidance over the current managed, user, and project skill inventory
    - explain eligibility, visibility, shadowing, first-use guidance, and product-mode fit rather than requiring operators to know a `skill_id` up front
    - keep install and invoke explicit and governed instead of drifting into blind auto-install
  - scoped memory retrieval productization:
    - add query-aware retrieval and broaden beyond session-summary-only hydration
    - make provenance and injection reason operator-visible
    - ship local text search before embedding-dependent retrieval

#### Browser And Product Surfaces

- managed browser automation companion:
  - keep `browser.open`, `browser.extract`, and `browser.click` as the shipped safe browser lane
  - partial governed adapter skeleton now exists for richer page actions:
    `browser.companion.*` becomes runtime-visible only when the companion is ready, read actions stay in the Core lane, and write actions stay in the governed App lane
  - still wire install, `onboard`, and `doctor` into companion presence, version, and isolated profile health
  - still add isolated browser profile lifecycle and release packaging around the companion runtime
  - keep richer browser automation exposed only through truthful runtime-visible tool advertising and governed tool contracts
- browser-facing product surface:
  - Web UI implementation as a thin shell over the local product control plane plus existing ask/chat, onboarding, dashboard, and browser semantics, not a separate assistant runtime
  - current product mode stays same-origin and localhost-only by default, but
    that operating boundary is not the long-term architecture endpoint

#### Gateway-Owned Service Foundation

- gateway service foundation:
  - land the first explicit daemon-owned gateway owner contract through
    `gateway run`, `gateway status`, and `gateway stop`, while keeping
    `channels serve` as the durable operator surface instead of carrying a
    compatibility-only runtime-owner noun at the root
  - extract channel, ACP, and runtime snapshot payload builders into shared
    service read models that can feed CLI, dashboard, Web UI, and future
    paired/browser/mobile clients
  - centralize bind ownership, route mounting, local admin auth, pairing, and
    detached service lifecycle in the gateway while preserving kernel, app, and
    ACP boundaries
  - use the gateway layer as the prerequisite for richer long-lived runtimes
    such as Discord, Slack, WhatsApp, and other gateway-native channel surfaces

Acceptance criteria:

- a new user can install and complete a first successful `ask` or `chat` in <= 5 minutes
- local memory persistence is stable across process restarts
- shell/file/web/browser tools obey policy constraints and emit auditable outcomes
- advertised tools match the actually invokable runtime surface for the current config and compiled features
- channel/provider modules can be toggled by feature flags without core code edits
- service-oriented product surfaces can converge on one daemon-owned gateway
  host without introducing a second assistant runtime or weakening kernel/app
  governance

## Quality Gate Matrix (Always On)

All roadmap stages must keep these gates green:

1. `cargo fmt`
2. `cargo test` (workspace full pass)
3. Security regression set (approval, scan, bridge constraints)
4. Audit schema stability checks for critical event kinds
5. No hardcoded risk exceptions when config-driven alternatives exist

## Discussion: Post-MVP Foundation Items

These items emerged from the Phase 0–3 restructure (PR #15). This section is a
public cross-cutting tail, not the full internal backlog. Keep it short and use
it only when the direction still matters to source readers.

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

Status (`dev`): implemented.

Why this note still matters:
- it was the first full provider-side context assembly slice to move from
  direct SQLite coupling onto a kernel-routed seam
- future compaction, context engines, and multi-agent context work should build
  on that seam instead of reintroducing hidden direct reads

Delivered:
- `ConversationContextEngine` seam plus registry and selector hooks
- kernel-routed memory-window reads when kernel authority is present
- rollback and diagnostics support through built-in `legacy` engine,
  observability commands, and runtime snapshot reporting
- reserved lifecycle and compaction hooks so future context evolution does not
  require another trait rewrite
- regression coverage for kernel-routed reads, runtime injection, and registry
  resolution

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

### D8: Local product control plane foundation

Loong now has enough real runtime substrate that the next platform risk is
surface drift rather than missing primitives.

Current baseline:

- a real ACP control plane
- a durable session repository
- operator-facing `onboard`, `doctor`, `runtime acp status`, and observability surfaces

Missing contract:

- one localhost-only product control plane that future HTTP and Web UI work can
  consume

Risk if skipped:

- browser-only runtime semantics
- a gateway-local session model
- a giant product gateway that starts stealing authority from the kernel

Preferred path:

- keep the kernel as authority
- keep ACP internal and real
- use `SessionRepository` as the canonical product session plane
- extract a shared local control plane for status, sessions, approvals, support
  flows, and future turn submission

Trade-off: this adds one explicit platform layer, but it prevents duplicated
surface logic and keeps future gateway/UI work aligned with the kernel-first
architecture.

### D9: Shared execution security tiers

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

### D10: First-party workflow packs on hardened primitives

Once the runtime base is harder, Loong should turn that into a small set of first-party
workflow packs that prove the kernel's value in operator-facing tasks such as release/review work,
issue triage, or channel support.

Trade-off: this is the right productization direction, but it should follow runtime hardening
instead of preceding it.

## Current Priority Order

1. Kernel-first runtime closure and direct-path retirement
2. Persistent audit sink and query baseline
3. ACP control-plane hardening and recovery
4. Local product control plane foundation
5. Shared execution security tiers across process/browser/WASM lanes
6. First-party workflow packs on hardened runtime primitives

Execution package for this order:

- the public roadmap above remains the OSS-facing source of truth for priority order
- the deeper implementation packages and internal backlog artifacts now live outside the public repository docs flow
