# Security

Security domain index for LoongClaw. For vulnerability reporting, see [SECURITY.md](../SECURITY.md) at repository root.

The public reader-facing summary for this material lives in
[`../site/reference/security-and-reliability.mdx`](../site/reference/security-and-reliability.mdx).
This file remains the repository-native security reference for source readers,
contributors, and maintainers.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read the public summary first | [`../site/reference/security-and-reliability.mdx`](../site/reference/security-and-reliability.mdx) |
| report a vulnerability privately | [`../SECURITY.md`](../SECURITY.md) |
| inspect the source-level security model in the repository | this file |
| understand the broader repository docs layering | [`README.md`](README.md) |

## Read This File When

- you need the repository-native security model and enforcement points
- you are reviewing whether a change weakens a fail-closed boundary
- you need the source-level hardening notes behind the public security summary

## Security Areas

| Area | What this file covers |
| --- | --- |
| security model | layered security assumptions and current enforcement status |
| enforcement points | policy, capability, audit, and runtime-boundary behavior |
| operator posture | `loong doctor security` and current local exposure review |
| outbound and execution trust boundaries | SSRF, execution tiers, and related runtime guardrails |

## Section Map

| Section | Read it when... |
| --- | --- |
| [Security Model](#security-model) | you need the layered security assumptions and current enforcement status |
| [Enforcement Points](#enforcement-points) | you need to review fail-closed boundaries in concrete runtime paths |
| [Audit System](#audit-system) | you need the retained-audit expectations behind operator review and debugging |
| [Operator Security Posture Audit](#operator-security-posture-audit) | you need the local exposure-review contract |
| [Web HTTP SSRF Guardrails](#web-http-ssrf-guardrails) | you need outbound HTTP trust-boundary details |
| [Shared Execution Security Tiers](#shared-execution-security-tiers) | you need the cross-lane execution-tier vocabulary |

## Security Model

LoongClaw implements a multi-layer security model. Higher layers add defense-in-depth:

| Layer | Mechanism | Version | Status |
|-------|-----------|---------|--------|
| 0 | Rust memory safety (compile-time, zero overhead) | v0.1 | Enforced |
| 1 | Capability-based access (type-system tokens) | v0.1 | Enforced |
| 2 | Namespace confinement (per-task resource view) | v0.1 | Struct exists, not enforced |
| 3 | WASM linear memory sandbox | v0.2 | Research only |
| 4 | Process isolation (seccomp+Landlock / restricted child) | v0.1 | Not implemented |

## Enforcement Points

### Policy Engine (L1)

Every kernel-bound core tool call passes through capability + policy gates:

```
CapabilityToken → PolicyEngine.authorize(...) → PolicyExtensionChain → Execution → Audit
```

Tool-specific request approval currently lives in the `PolicyExtensionChain`; the legacy
`PolicyEngine::check_tool_call` hook is deprecated.

### Current Coverage

The L1 chain already covers most reader-visible runtime paths. Read the group
that matches the surface you are changing instead of treating this as one flat
inventory.

#### Core Runtime And Tool Surfaces

- `shell.exec` — kernel-mediated core tool execution with capability checks,
  shell policy extensions, and audit events
- `file.read` / `file.write` / `file.edit` — kernel-mediated core tool
  execution with filesystem capabilities, file policy extension checks,
  execution-layer path sandboxing, and audit events
- Conversation tool turns — fast-lane and safe-lane inner tool execution now
  flow through explicit `ConversationRuntimeBinding` (`Kernel` or `Direct`);
  missing kernel authority fails closed as `no_kernel_context`, and async
  delegate children inherit parent kernel authority instead of forcing direct
  mode
- Memory/runtime/context orchestration — the conversation module now carries
  `ConversationRuntimeBinding` end-to-end across runtime, context,
  persistence, turn coordination, loop followup, history, and app-dispatch
  seams; kernel-bound history readers fail closed on kernel memory-window
  errors or non-`ok` statuses instead of silently downgrading to direct sqlite
- Provider request/failover orchestration — provider request entrypoints and
  failover telemetry now use explicit `ProviderRuntimeBinding` (`Kernel` or
  `Direct`); failover metrics record in both modes, but kernel-backed audit
  emission only occurs when provider execution is explicitly kernel-bound
- Outer integration wrappers — raw optional kernel context is now limited to
  explicit integration boundaries such as
  `channel::process_inbound_with_provider`, which normalize into a
  binding-first runtime seam instead of carrying shadow authority semantics
  deeper into the runtime

#### Plugin Intake And Compatibility

- Plugin intake and ownership — `PluginManifest::slot_claims` now enforces
  exclusive/shared/advisory ownership boundaries during absorb. Conflicting
  claims surface as blocked activation candidates before bootstrap where
  possible, still fail before catalog mutation if they reach absorb, and
  normalized claim metadata now projects into provider inventory,
  self-awareness activation inventory, plus spec `plugin_inventory` /
  `tool_search` visibility
- Plugin host compatibility — `PluginManifest::compatibility` now lets
  packages declare supported host API and version requirements. Invalid or
  incompatible declarations fail closed during absorb, surface as
  `blocked_incompatible_host` during activation planning, and project into
  operator-facing inventory so incompatible plugins stay discoverable without
  becoming active by accident
- Plugin package contract strictness — package manifests now require explicit
  `api_version` plus top-level `version`, reject unknown top-level fields,
  reject legacy `metadata.version`, reserve the `plugin_*` metadata namespace
  for host-managed projection, and validate typed plugin `version` as semver
- Plugin diagnostics contract — plugin scan warnings and activation blockers
  now project as structured diagnostics with stable codes, severity, lifecycle
  phase, blocking truth, field paths, and remediation hints across
  inventory/search/self-awareness surfaces

#### Preflight Governance And Operator Automation

- Plugin preflight governance — the spec/runtime layer now exposes a
  profile-aware `plugin_preflight` surface that reuses the same diagnostics and
  activation truth for current-host activation gates, SDK release gates, and
  stricter marketplace submission gates instead of letting each external tool
  invent its own policy parser
- Policy integrity and traceability — plugin preflight governance policies can
  now be loaded from external JSON, pinned by sha256, optionally
  signature-verified, and echoed back through policy source/version/checksum
  metadata. Explicitly requested custom policies fail closed on load or
  validation errors instead of silently downgrading to the bundled policy
- Structured governance actions — plugin preflight recommended actions now also
  carry a typed `operator_action` envelope with a stable `action_id`, owning
  surface, action kind, target plugin/provider identity, reload requirement,
  and follow-up preflight profile
- Governance workload aggregation — plugin preflight summary output now also
  emits a deduplicated `operator_action_plan` and counts distinct operator
  actions by surface, kind, and reload requirement
- Thin operator CLI boundary — `loongclaw plugins preflight` and
  `loongclaw plugins actions` intentionally consume the existing
  `plugin_preflight` spec contract and its structured summary/action-plan
  output instead of embedding a second policy evaluator in the operator layer
- Bundled compatibility presets — plugin governance now also exposes
  checksum-pinnable bundled bridge support profiles such as
  `native-balanced` and `openclaw-ecosystem-balanced`
- Discoverable preset surface — `loongclaw plugins bridge-profiles` now
  exposes those bundled compatibility presets and their exact
  bridge/shim/language support matrix through one bounded operator CLI
- Machine-readable remediation contract — plugin preflight results now carry
  typed remediation classes plus structured recommended actions, so SDK
  packaging flows, CI, and marketplace moderation can automate on one stable
  remediation vocabulary
- Baseline versus effective governance truth — plugin preflight now reports
  both baseline verdicts and effective post-policy outcomes, including waived
  flags/codes and summary-level baseline counters

#### Runtime Attestation, Exceptions, And Observability

- Activation attestation boundary — ready plugin absorbs now stamp a
  checksum-pinned `plugin_activation_contract_json` into provider metadata, and
  runtime bridge dispatch verifies both the attested checksum and the current
  provider projection before allowing foreign-compatibility execution
- Operator-facing attestation truth — loaded-plugin `tool_search`,
  `plugin_inventory`, and `plugin_preflight` surfaces now expose whether
  activation attestation is `verified`, `missing`, or `invalid`, including
  checksum drift context when available
- Ecosystem observability rollups — plugin preflight summary output now also
  aggregates source kind, dialect, compatibility mode, source language, and
  bridge kind distributions so operators can spot foreign-dialect or
  shim-profile expansion from one bounded report
- Contract-drift exception lane only — custom preflight policy exceptions are
  explicit and auditable, but are intentionally limited to manifest/package
  drift findings; they cannot waive activation blockers, unsupported runtime
  bridges, slot ownership conflicts, or other kernel fail-closed boundaries
- Version-scoped and approval-attested waivers — every preflight exception can
  now be narrowed to a plugin semver range and must carry stable `ticket_ref`
  plus `approved_by` metadata

#### Known Gap

- Connector/ACP/runtime-only analytics — not uniformly routed through the L1
  policy chain yet

#### Binding Notes

Conversation runtime binding:

- `Kernel` means the turn is allowed to call kernel-mediated core tools;
  `Direct` means conversation orchestration may continue, but kernel-only tool
  execution must fail closed
- it removes ambiguity from conversation traits and dispatcher seams where
  `None` previously overloaded "direct mode", "not wired yet", and "forgot to
  pass kernel authority"
- detached async delegate spawns carry an owned kernel context forward when the
  parent binding is kernel-bound; direct-mode parents keep direct-mode children
- kernel-bound history helpers no longer reuse direct sqlite fallback behind
  the caller's back, and diagnostics now surface explicit history load status
  plus normalized error codes
- user-facing chat diagnostics and the discovery-first session-history path now
  preserve explicit `ConversationRuntimeBinding` semantics end-to-end

Provider runtime binding:

- `Kernel` means failover/audit behavior may emit kernel-backed audit events;
  `Direct` means provider execution is intentionally running without that
  authority while still recording in-process failover metrics
- this keeps provider governance explicit without importing conversation-layer
  semantics into provider code

### Capability Tokens

- 8 capability types with generation-based revocation
- `AtomicU64` threshold: revoke all tokens with generation <= N
- TTL enforcement on every authorization check
- `membrane` field exists but not enforced (TD-003)

### Audit System

- 7 event kinds with atomic sequencing
- Production app runtimes default to durable JSONL retention via `[audit].mode = "fanout"`
- Default journal path: `~/.loongclaw/audit/events.jsonl`
- `LoongClawKernel::new()` and spec/test/demo helpers may still opt into explicit in-memory audit seams when side-effect-free snapshot reporting is required
- Explicit no-audit behavior remains opt-in only and should stay reserved for narrow fixture seams
- No HMAC chain for tamper evidence (TD-007)

### Operator Security Posture Audit

- `loong doctor security` is the operator-facing exposure audit for the current local runtime
- It reports `covered`, `partial`, `exposed`, and `unknown` findings instead of collapsing everything into pass/fail
- Current audit categories include durable audit retention, shell posture, tool file-root confinement, web-fetch egress, external-skills posture, secret hygiene, and browser automation surfaces
- `loong doctor security --json` provides a stable machine-readable report for local automation and support workflows

### Web HTTP SSRF Guardrails

- `web.fetch`, `web.search`, and the shared browser-side URL validators intentionally build their HTTP clients with `reqwest::ClientBuilder::no_proxy()`
- This keeps DNS resolution and connect-time routing inside the same SSRF-safe policy boundary instead of delegating host decisions to ambient `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, or `NO_PROXY` environment settings
- The built-in browser surface now uses the same SSRF-safe client construction, so browser navigation no longer trusts ambient proxy environment variables for host reachability decisions
- The managed browser companion validates both the requested navigation target and the returned `page_url` against its runtime web policy, and it tears down companion session state if a returned `page_url` falls outside that policy
- Config-backed outbound channel endpoints now share one outbound HTTP trust policy: URLs must use `http` or `https`, must not embed credentials, and block private or special-use hosts by default
- Channel outbound HTTP clients do not auto-follow redirects, which prevents an initially trusted endpoint from silently crossing into a different destination after the first response
- Operators who intentionally target a local or private bridge can widen that boundary with `[outbound_http] allow_private_hosts = true`; the default remains fail-closed for public-only outbound delivery
- Trade-off: corporate proxy-only egress is not currently supported for these web tools because a proxy hop would weaken the same-host assumptions behind the private-address guard
- If proxy-aware web tooling is added later, it should preserve the same SSRF guarantees rather than silently bypassing them

### Shared Execution Security Tiers

LoongClaw now uses one shared execution-tier vocabulary across the process, browser, and WASM
lanes. The first slice standardizes the contract and the emitted evidence; it does not attempt a
full sandbox rewrite for every lane at once.

| Tier | Meaning | Current lane mapping |
|------|---------|----------------------|
| `restricted` | tightly bounded execution intended for untrusted or heavily constrained work | built-in browser tools and the current WASM component runtime lane |
| `balanced` | richer operator-governed execution with explicit readiness or allowlist gates | allowlisted `process_stdio` bridge execution and the managed browser companion when its runtime gate is open |
| `trusted` | reserved for future explicit high-trust execution lanes | no default lane maps here yet |

Current evidence surfaces that emit or expose this vocabulary:

- `process_stdio` bridge runtime evidence now includes `execution_tier`
- WASM bridge runtime evidence now includes `execution_tier`
- browser tool payloads and runtime snapshots now include `execution_tier`

### Compile-Time Constraints

25 workspace clippy denies prevent common agent anti-patterns. See [Harness Engineering](design-docs/harness-engineering.md) for the full list.

## See Also

- [Design Docs Index](design-docs/index.md) — security-related design decisions
- [Layered Kernel Design](design-docs/layered-kernel-design.md) — L1 security layer specification
- [Core Beliefs](design-docs/core-beliefs.md) — principle #3: capability-gated by default
