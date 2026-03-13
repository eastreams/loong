# LoongClaw Kernel Layered Design (v0.1)

This document defines the low-level layering model for a minimal but extensible Agentic OS kernel.

## Design Targets

- Keep the kernel small and deterministic.
- Treat security as a default boundary, not an addon.
- Allow fast specialization without mutating core contracts.
- Enable bidirectional integration (others integrate us, we integrate others).
- Keep architecture testable with deterministic, layered test suites.

## Layer Map

### L0. Contract Layer (Kernel ABI Surface)

Scope:

- `contracts.rs`
- stable request/outcome structs
- capability model and route model

Rules:

- Backward compatibility first.
- No domain-specific semantics in L0.
- Serialization format and field behavior are part of the kernel contract.

### L1. Security and Governance Layer

Scope:

- `policy.rs` (core policy engine)
- `policy_ext.rs` (environment/domain policy overlays)
- capability token issue/revoke/authorize lifecycle

Rules:

- Every external action must pass L1.
- Tool plane core/extension execution must call `PolicyEngine::check_tool_call` before dispatch
  (Rule of Two: model intent plus deterministic policy decision).
- Policy extensions can only tighten behavior, never weaken core policy.
- Denials are auditable and deterministic.
- Human approval gate should default to medium-balanced mode:
  high-risk tool calls require explicit user authorization, while low-risk calls stay fast.
- Approval strategy must be configurable between per-call authorization and one-time full-access grant.
- Risk detection signals/scoring must be profile-driven (external JSON), with inline overrides only as
  temporary overlays to avoid hardcoded policy drift.
- Plugin security scan must support a hard gate (`block_on_high`) and structured evidence output
  so risky community plugins never silently enter the runtime catalog.
- External security scan profiles should support optional checksum pinning
  (`security_scan.profile_sha256`) so profile tampering fails closed.
- External security scan profiles should support optional signature verification
  (`security_scan.profile_signature`) for key-based integrity validation.
- Security scan evaluation must be emitted as a typed audit event (`SecurityScanEvaluated`) so
  governance systems can consume findings without parsing ad-hoc report text.
- Security scan findings should support deterministic correlation IDs and optional JSONL SIEM export
  (`security_scan.siem_export`) with configurable fail-closed behavior.
- WASM plugin path should be treated as the preferred untrusted-extension lane, with static checks
  for artifact path scope, module size, hash pin, and import policy before absorb/hotplug.
- Denylist must have highest precedence over allowlist/full-access grants.
- One-time full-access grants should support expiry and remaining-use limits to reduce blast radius.

### L2. Execution Plane Layer (Core + Extension Split)

Scope:

- `runtime.rs`
- `tool.rs`
- `memory.rs`
- `connector.rs`

Pattern:

- `Core*Adapter`: minimal trusted substrate.
- `*ExtensionAdapter`: rich behavior composed over the core adapter.
- `*Plane`: adapter registry, default-core selection, and dispatch.

Rules:

- Extension path never bypasses core path.
- Core interfaces remain stable and minimal.
- New capabilities prefer extension adapters over core contract growth.
- Each plane supports explicit default-core selection to make orchestration deterministic.

### L3. Orchestration Layer

Scope:

- `harness.rs`
- `kernel.rs`

Responsibilities:

- Route task execution to harness adapters.
- Enforce pack boundaries and capability boundaries.
- Bridge L1 policy decisions to L2 execution.
- Emit lifecycle audit events.

Rules:

- Orchestrator is policy-aware but business-logic-light.
- All plane calls are gated by the same pack/policy checks.

### L4. Observability and Determinism Layer

Scope:

- `audit.rs`
- `clock.rs`

Responsibilities:

- Event timeline abstraction.
- Sink abstraction for durable audit integration.
- Deterministic clocking for reproducible tests.
- Unified `PlaneInvoked` evidence for runtime/tool/memory/connector execution paths.

Rules:

- No direct dependency on wall clock in tests.
- Security-critical decisions must produce auditable event evidence.

### L5. Specialization Layer (Vertical Pack)

Scope:

- `pack.rs`

Responsibilities:

- Domain packaging contract (`pack_id`, version, capabilities, allowed connectors, default route).
- Boundary declaration for what one vertical pack can or cannot do.

Rules:

- Specialization is data/config-driven through pack manifests.
- Core kernel logic should not fork per vertical domain.

### L5.5 Protocol Foundation Layer

Scope:

- `crates/protocol`
- transport frame contracts
- typed protocol method router
- in-memory linked channel transport primitive
- json-line stream transport primitive for stdio/pipe integration

Responsibilities:

- Keep protocol transport and routing contracts out of daemon business logic.
- Provide typed route resolution for standard methods before handler dispatch.
- Provide deterministic local transport primitive for test/runtime bridging.

Rules:

- Unknown custom methods must fail closed in strict mode.
- Transport close semantics must be explicit and testable.
- Bounded queues must preserve backpressure instead of unbounded buffering.
- Runtime process stdio execution should consume protocol transport primitives
  (json-line frame contract) instead of ad-hoc stdin/stdout JSON handling.
- Runtime process stdio execution should enforce protocol-route authorization,
  request/response method+id consistency, and bounded timeout controls.
- Runtime http_json execution should enforce protocol-route authorization and
  support optional strict method/id response contract validation.
- Shared protocol-context construction should be reused across bridge executors
  to avoid policy drift between transport implementations.
- Shared protocol runtime-evidence field appending should be reused across
  bridge executors to keep telemetry schema stable and comparable.
- Bridge protocol helper logic should be isolated in a dedicated module include
  to avoid unchecked line-count growth in the runtime orchestration file.
- Bridge regression suites should be organized in dedicated test modules so
  protocol-contract and authorization assertions remain maintainable.
- Bridge runtime telemetry should be emitted through typed evidence structs and
  shared serialization to keep executor payload shape evolution controlled.
- Runtime evidence builders should use explicit state variants rather than
  wide optional-field bags, and tests should assert exact key sets per state.

### L6. Integration Control Plane (Autonomous Provisioning)

Scope:

- `integration` catalog and auto-provision planner
- provider/channel hotplug and hotfix workflows
- plugin scanner for source-driven community extension ingestion

Responsibilities:

- Detect missing provider/channel requirements and synthesize provisioning plans.
- Apply plan atomically to integration catalog and pack boundaries.
- Support runtime hotfix (provider version, connector remap, channel endpoint, channel enablement).
- Scan existing source files and absorb embedded plugin manifests into live integration state.
- Keep plugin ingestion language-agnostic by relying on marker-delimited JSON manifests in source comments.

Rules:

- Auto-provision may only expand pack boundaries explicitly (connector allowlist and required capabilities).
- Hotfix operations must be auditable and reversible through catalog snapshots.

### L7. Plugin Translation Plane (Multi-Language IR)

Scope:

- `plugin_ir` canonical representation
- bridge-kind inference (`http_json`, `process_stdio`, `native_ffi`, `wasm_component`, `mcp_server`, `acp_bridge`, `acp_runtime`)
- adapter family / entrypoint hint normalization

Responsibilities:

- Convert language-specific plugin manifests into a stable, language-agnostic IR contract.
- Decouple community plugin authoring language from kernel integration workflow.
- Provide deterministic metadata for runtime bridge selection and future auto-wiring.
- Produce activation plans against a declared bridge support matrix before plugin hotplug.

Rules:

- Translation must be deterministic and reproducible from source manifests.
- Metadata overrides are allowed only through explicit manifest fields.
- IR output is informational/control data and must not bypass policy boundaries.
- When strict bridge enforcement is enabled, unsupported bridge/adapter profiles block hotplug.
- When strict bridge enforcement is disabled, unsupported plugins are skipped while ready plugins are still absorbable.
- Bridge policy supports optional integrity pinning via checksum and SHA256 digest
  to prevent silent policy drift.
- Bridge policy can optionally enable local bridge runtime execution in controlled mode
  (`process_stdio` allowlist and strict execution enforcement).
- ACP-related bridge taxonomy must preserve the distinction between an ACP bridge surface
  (`acp_bridge`) and a session-aware ACP runtime backend (`acp_runtime`), so runtime backends such
  as ACPX do not collapse into the same abstraction bucket as bridge/gateway entrypoints.
- WASM runtime execution is policy-driven through `security_scan.runtime` with fail-closed
  guards (`execute_wasm_component`, `allowed_path_prefixes`, `max_component_bytes`,
  `fuel_limit`) so enabling execution never requires hardcoded kernel branches.

### L8. Self-Awareness and Architecture Guard Plane

Scope:

- `awareness` snapshot builder
- `architecture` immutable-core guard policy

Responsibilities:

- Build deterministic codebase snapshots (language distribution, plugin inventory, file fingerprint).
- Evaluate proposed mutation paths against immutable-core and mutable-extension boundaries.
- Provide pre-execution guard decisions so agents cannot mutate critical kernel contracts silently.

Rules:

- Unknown mutation paths are denied by default in strict guard mode.
- Immutable-core boundaries are explicit and reviewable.
- Guard enforcement decisions are serializable and testable.

### L9. Bootstrap Execution Plane

Scope:

- `bootstrap` executor and policy
- ready-plugin apply/defer/skipped lifecycle

Responsibilities:

- Convert plugin activation outcomes into executable bootstrap tasks.
- Apply only policy-allowed ready plugin tasks (`applied`) and keep explicit reasons for deferred/skipped tasks.
- Optionally enforce hard gate when any ready plugin cannot be auto-applied.

Rules:

- Bootstrap executor never widens policy or capability boundaries.
- Absorb into integration catalog/pack must only use bootstrap-`applied` plugin set when bootstrap is enabled.
- Multi-root plugin bootstrap/absorb should be transactional to avoid partial commit under blocked states.
- Bootstrap `max_tasks` should be interpreted as a run-level budget across all scan roots.
- Deferred/skipped tasks must remain observable for follow-up orchestration.
- ACP-related bootstrap policy must preserve separate auto-apply gates for bridge surfaces and
  runtime-backend surfaces, so `acp_bridge` rollout and `acp_runtime` rollout can be governed
  independently.
- Applied plugins should expose a normalized bridge execution contract (`bridge_execution`) so runtime
  behavior remains deterministic and inspectable after hotplug.
- Local bridge execution must be opt-in and allowlisted; default mode remains plan-only.

## Why More Than Runtime/Tool Need Layering

Runtime and tool are only part of extension pressure. The same pressure exists in:

- connector integration (third-party diversity and protocol drift)
- policy behavior (environmental hardening and compliance)
- memory strategy (storage substrate vs semantic retrieval enrichment)
- observability sinks (local, SIEM, compliance pipelines)

Without layering these modules, the kernel becomes a monolith and loses long-term stability.

## Layered Testing Strategy

### T0. Contract Tests

- Semver and manifest validation.
- Struct-level serialization/deserialization invariants.

### T1. Security Invariant Tests

- Capability boundary checks.
- Token expiry and revocation.
- Policy-extension denial paths.

### T2. Plane Conformance Tests

- Core adapter dispatch per plane.
- Extension-over-core composition per plane.
- Missing adapter/default adapter error behavior.

### T3. Orchestration Tests

- Pack + policy + plane integration.
- Harness route selection.
- Connector whitelist gating.

### T4. Audit and Determinism Tests

- Event emission completeness on success/denial/revocation.
- Fixed clock reproducibility.
- Golden audit schema assertions for critical event contracts.

### T5. Scenario/Smoke Tests

- `daemon` command-level integration smoke.
- Representative vertical-pack execution flows.
- Runtime isolation smoke for `wasm_component` execution (success + path/size guard denials).

### T6. Property Tests

- Generated capability-set combinations to validate pack boundary invariants.

### T7. Self-Governance Tests

- Awareness snapshot determinism and language inventory checks.
- Architecture guard denial paths for immutable-core mutation proposals.
- Plugin IR translation consistency across multi-language plugin descriptors.
- Tool discovery consistency across absorbed and deferred plugin sets.
- Programmatic orchestration policy tests (connector allowlist, call budget, caller ACL,
  batch parallel/continue-on-error behavior, conditional branch predicates, retry/jitter,
  rate shaping, circuit breaker open/invalid-policy paths, and policy-driven adaptive
  concurrency triggers).

## Near-Term Evolution Plan

1. Add per-plane conformance test templates and macro helpers to reduce boilerplate.
2. Add deterministic fault-injection adapters for connector/runtime/tool/memory.
3. Expand golden event tests for additional audit schemas.
4. Expand property-based tests for capability/pack boundary combinations.
5. Add compatibility tests for future contract revisions.
6. Expand timing-sensitive integration tests for circuit half-open recovery and adaptive concurrency behavior under mixed failure/latency workloads.
