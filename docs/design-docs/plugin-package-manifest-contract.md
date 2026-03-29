# Plugin Package Manifest And Setup Contract

## Purpose

LoongClaw already has a real plugin intake path:

- source scanning through `PluginScanner`
- bridge/runtime translation through `PluginTranslator`
- activation planning through `PluginActivationPlan`
- policy-bounded apply/defer decisions through `PluginBootstrapExecutor`

That baseline is useful, but it is still one layer short of a durable ecosystem
contract. This document defines the next contract layer:

- manifest-first package metadata
- setup-only plugin surfaces for onboarding and doctor
- capability-slot ownership semantics for plugin-provided runtime surfaces
- host compatibility fences for future SDK and runtime evolution

The intent is to learn from OpenClaw's manifest-first shape without copying its
in-process trust model.

## Why This Contract Exists

The current `PluginManifest` path in `crates/kernel/src/plugin.rs` is still
source-oriented:

- manifests are discovered from marker-delimited comment blocks
- discovery depends on parsing source files instead of package metadata
- setup and onboarding cannot consume plugin metadata without scanning source
- ownership conflicts are inferred indirectly from ids instead of explicit slot
  semantics

That shape is acceptable while plugin intake remains internal or experimental.
It will not scale cleanly to:

- third-party package distribution
- setup-time provider/channel guidance
- manifest-driven plugin catalogs
- deterministic conflict handling for shared vs exclusive plugin surfaces
- host/runtime evolution where older plugins need an explicit compatibility
  contract instead of optimistic best-effort loading

OpenClaw's best lesson here is structural, not runtime-specific:

- package metadata should be a first-class contract
- setup should be separable from full runtime activation
- ownership semantics should be explicit instead of implicit

LoongClaw should absorb those lessons while preserving its stronger
kernel-governed safety boundary.

## Current Baseline

Today LoongClaw already proves several important building blocks:

- `PluginManifest` carries typed identity and metadata
- `PluginIR` normalizes multi-language plugin intake into a bridge-neutral form
- `BridgeSupportMatrix` blocks unsupported bridge, adapter, compatibility-mode,
  and compatibility-shim profiles
- `BootstrapPolicy` keeps plugin activation policy-driven and auditable
- Roadmap stages already call for community plugin intake, signing, and trust
  tiers

The missing piece is the package contract that sits before translation and
before bootstrap.

## Delivered First Slice

The current dev baseline now includes the first enforceable slice of this
contract:

- `PluginManifest` carries typed `slot_claims` entries with `slot`, `key`, and
  `mode`
- `PluginManifest` also carries optional typed `compatibility` metadata with
  `host_api` and `host_version_req`
- package manifests now treat `api_version` as an explicit schema contract
  instead of silently ignoring it
- package manifests now require typed top-level `version`; legacy
  `metadata.version` remains a source-marker migration path instead of a public
  package contract
- typed plugin `version` now normalizes into manifest truth and inventory
  surfaces instead of living only in ad hoc metadata
- manifest normalization trims, sorts, and deduplicates slot claims before
  downstream comparison
- compatibility metadata is normalized and compared during package-manifest
  precedence resolution so package vs source drift fails deterministically
- package manifests now fail closed on unknown top-level fields so SDK and
  marketplace authors do not accidentally ship misspelled contract keys that
  the runtime silently ignores
- package manifests now also reserve the `plugin_*` metadata namespace for
  host-managed projection so third-party packages cannot collide with operator
  inventory, bootstrap, or activation metadata
- package-manifest precedence checks now treat conflicting `slot_claims` as a
  deterministic manifest conflict instead of silently drifting
- package/source precedence now also treats conflicting `api_version` and
  `version` as deterministic manifest drift
- incompatible `compatibility` declarations now fail closed during absorb
  before catalog mutation and block activation planning before slot/bridge
  selection
- `PluginScanner::absorb` enforces slot ownership before catalog mutation:
  - blank slot identifiers fail closed
  - one plugin cannot declare the same slot/key pair with conflicting modes
  - `exclusive` conflicts with any other plugin that already claims the same
    slot/key pair
  - `shared` and `advisory` may coexist
- activation planning now surfaces slot-claim conflicts as blocked candidates
  before bootstrap/apply, including conflicts inside the scanned plugin set and
  conflicts against already-registered provider inventory when claim metadata is
  present
- absorbed provider metadata now projects `plugin_slot_claims_json` so spec and
  operator-facing discovery surfaces can render ownership boundaries without
  reparsing source
- absorbed provider metadata also projects compatibility metadata so future
  doctor/onboard/catalog surfaces can reason about host fences without
  rehydrating source manifests
- ready plugin absorbs now also stamp a checksum-pinned activation runtime
  contract into provider metadata so runtime dispatch can verify "what was
  approved then" against "what still exists now" without reparsing source
- loaded plugin inventory and discovery surfaces now also expose activation
  attestation integrity directly, so operators and future SDK or marketplace
  tooling can tell whether a loaded plugin still proves the contract it crossed
  activation with instead of assuming absorb success is still valid
- `tool_search` now carries both slot claims and activation status/reason so
  product-facing discovery can show "present but blocked" plugins truthfully
- `tool_search` and `plugin_inventory` now both surface compatibility truth so
  operators can distinguish "discoverable but incompatible with this host" from
  bridge/runtime blockers
- scan, translation, activation, and operator inventory/search surfaces now
  share stable structured plugin diagnostics with machine-readable codes,
  severity, lifecycle phase, blocking truth, field paths, and remediation hints
  so SDK tooling, CI, and future marketplace review do not need to parse
  free-form error strings or reverse-engineer whether a finding is merely
  advisory versus activation-blocking
- self-awareness snapshots now carry per-plugin activation inventory so future
  doctor/onboard/inventory surfaces can reuse the same boundary truth instead
  of re-deriving it ad hoc
- spec execution now exposes a dedicated `plugin_inventory` operation for
  plugin-level operator surfaces, instead of forcing everything through the
  tool-centric search view
- spec execution now also exposes `plugin_preflight`, a profile-aware
  governance surface that evaluates the same plugin truth against
  `runtime_activation`, `sdk_release`, and `marketplace_submission` gate
  profiles so CI, SDK packaging flows, and future marketplace review can share
  one reusable contract
- `plugin_preflight` policy is now externalizable and integrity-pinnable:
  callers can reuse the bundled balanced policy or point at a custom JSON
  policy file with sha256 and optional signature verification, while results
  report the resolved policy source, checksum, sha256, and version for
  traceable governance; explicitly requested custom policies now fail closed if
  they cannot be loaded or validated instead of silently falling back
- `plugin_preflight` now also emits machine-readable `remediation_classes` and
  structured `recommended_actions` so SDK CLIs, CI gates, and future
  marketplace review backends can consume stable remediation vocabulary instead
  of scraping free-form summaries
- `recommended_actions` now also carry a stable `operator_action` envelope with
  stable `action_id`, execution surface, action kind, concrete plugin/provider
  targets, reload requirement, and optional follow-up preflight profile so
  future CLIs, UIs, doctor-like operator surfaces, and marketplace workflows
  can route the same governance truth without hard-coding remediation text
  parsing
- preflight summary output now also exposes a deduplicated
  `operator_action_plan` plus aggregates by owning surface and action kind, and
  reload-required versus no-reload counts, using distinct action targets rather
  than raw repeated messages, so dashboards and batch governance tooling can
  consume one stable worklist instead of replaying every individual plugin
  result
- the daemon now exposes `loongclaw plugins preflight` and
  `loongclaw plugins actions` as a thin operator shell over the same
  `plugin_preflight` spec execution contract, so future SDK, CI, registry, and
  marketplace tooling can share one governance truth instead of re-implementing
  policy evaluation inside each host-facing surface
- that operator shell now also supports bundled bridge support profiles such as
  `native-balanced` and `openclaw-ecosystem-balanced`, so hosts can opt into
  broader compatibility through explicit, checksum-pinnable presets instead of
  hand-rolling ad hoc runtime matrices in every integration
- the same operator surface now also exposes `loongclaw plugins bridge-profiles`
  so host automation can discover those bundled compatibility presets and their
  exact checksum-pinned contents without scraping docs or reverse-engineering
  source
- `plugin_preflight` now also treats invalid loaded activation attestation as a
  first-class runtime-activation blocker and emits a dedicated
  `quarantine_loaded_provider` plus `repair_runtime_attestation` remediation
  pair, so current-host readiness cannot silently pass on drifted or partially
  corrupted provider metadata
- `plugin_preflight` now distinguishes baseline governance truth from effective
  policy outcome through `baseline_verdict`, effective-vs-raw flags/diagnostic
  sets, and summary-level baseline counters so operators can see whether a
  plugin truly passed cleanly or only passed after an approved exception
- preflight policy files now support an explicit exception lane for contract
  drift only: exceptions are typed, pinned, auditable, and may waive manifest
  migration or metadata-cleanup findings, but they cannot waive activation
  blockers or other kernel fail-closed runtime boundaries
- contract-drift exceptions are now also version-scoped and approval-attested:
  policies may target a semver range such as `<0.2.0`, but every exception
  must carry stable `ticket_ref` and `approved_by` metadata so private
  registries, CI, and future marketplace review can distinguish a clean pass
  from a version-bound grandfathered pass
- preflight summaries now split `clean_passed_plugins` from
  `waived_passed_plugins` and aggregate exception usage by ticket and approver
  so operator dashboards can detect waiver sprawl instead of only counting raw
  passes
- preflight summaries now also aggregate ecosystem shape by source kind,
  dialect, compatibility mode, source language, and bridge kind so host
  operators can see whether their plugin surface is drifting toward foreign
  dialect, shim-heavy, or bridge-constrained mixes before those mixes become
  delivery or marketplace problems
- the daemon now also exposes `loongclaw plugins init <package_root>` as a
  bounded authoring bootstrap that writes a canonical
  `loongclaw.plugin.json` plus a README for external authors instead of asking
  them to reverse-engineer package shape from kernel internals or tests
- scaffolded manifests pin `api_version` and `compatibility.host_api` to the
  current host contract and reuse kernel-owned bridge defaults for
  `bridge_kind`, `adapter_family`, and `entrypoint`
- package-manifest runtime projection now also honors explicit
  `metadata.source_language`, so language-specific scaffolds do not collapse
  into a fake `manifest` language during preflight, bridge-profile fit
  analysis, or activation planning

This is intentionally not the full Stage 4 ecosystem model yet. It closes the
most important ownership ambiguity first: plugin packages can now express and
enforce runtime-surface intent as a kernel-visible contract.

## Non-Goals

This contract does not:

- switch LoongClaw to untrusted in-process native plugins by default
- replace kernel registry or policy ownership with plugin-owned runtime policy
- force every plugin onto the same runtime bridge
- solve marketplace distribution, signing, or supply-chain trust by itself
- replace the existing source-marker intake path in one breaking step

Those concerns are follow-on work. This contract exists so those later steps
share one metadata and ownership model.

## Contract

### 1. Package Manifest Owns Plugin Identity

Every distributable plugin package should have one package-level manifest file.

Recommended filename:

- `loongclaw.plugin.json`

The repo now also ships a bounded first-mile authoring command for that file
contract:

```bash
loongclaw plugins init ./plugins/tavily-search \
  --plugin-id tavily-search \
  --provider-id tavily \
  --connector-name tavily-http \
  --bridge-kind http_json \
  --summary "Tavily-backed search package"
```

The scaffold is intentionally narrow:

- it creates an empty package root only
- it writes a canonical `loongclaw.plugin.json`
- it writes a README that points authors to `plugins preflight` and
  `plugins actions`
- it does not generate runtime code, widen trust policy, or guess setup/slot
  ownership

The manifest is the source of truth for:

- canonical `plugin_id`
- version and display metadata
- provided runtime surfaces
- bridge/runtime metadata
- declared `trust_tier` classification metadata for operator-visible policy and review
- setup metadata
- capability-slot ownership declarations
- host compatibility declarations

Source-embedded marker blocks remain valid during migration, but they become a
compatibility input rather than the preferred contract.

### 2. Discovery Is Manifest-First And Additive

Discovery precedence should be:

1. package manifest file
2. embedded source manifest block

If both exist for the same package root:

- the package manifest is authoritative
- embedded source metadata may fill only explicitly-compatible optional fields
- conflicting values fail discovery with a typed reason instead of silently
  merging

This keeps the migration additive while preventing hidden package drift.

### 3. Setup Is A Separate Surface From Runtime Activation

Each plugin package may expose an optional `setup` section that is safe to
consume before the runtime bridge is activated.

The setup contract should support two modes:

- `metadata_only`
- `governed_entry`

`metadata_only` is the default and should carry:

- required environment variable names
- recommended environment variable names
- required config keys
- onboarding surface hints such as `web_search`, `channel`, or `memory`
- documentation links or remediation copy

`governed_entry` is optional and should:

- run through an explicit bridge contract
- respect the same policy and audit boundaries as any other plugin execution
- never imply in-process trust
- stay focused on setup/health actions rather than full runtime service

Onboarding, install, and doctor should be able to render setup guidance from
manifest metadata alone. Executing a governed setup entry should be an explicit
second step, not a prerequisite for discovery.

### 4. Ownership Uses Capability Slots, Not Hidden Conventions

Plugin packages should declare the runtime surfaces they own through explicit
slot declarations instead of only through loosely-related ids.

A slot declaration should contain:

- `slot`
- `key`
- `mode`

Recommended modes:

- `exclusive`
- `shared`
- `advisory`

Examples:

- `provider:web_search` + `tavily` + `exclusive`
- `channel:telegram` + `default` + `exclusive`
- `tool:search` + `web` + `shared`
- `memory:indexer` + `vector` + `advisory`

The important distinction is that raw capabilities and ownership are not the
same thing:

- capabilities describe what the plugin is allowed to do
- slots describe which runtime surface the plugin intends to own or extend

That separation prevents the registry and bootstrap layers from inferring
product ownership from low-level execution capabilities.

### 5. Registry Remains Kernel-Owned

The package manifest must feed the registry. It must not replace the registry.

Registry-owned behavior remains responsible for:

- canonical runtime ids
- effective selection order
- operator-facing grouped inventory
- final conflict resolution
- policy-bound activation state

Manifest data is inventory input. The kernel and registry remain the final
control plane for what becomes active.

### 6. Host Compatibility Is Explicit And Fail-Closed

Plugin packages should be able to declare which LoongClaw host contract they
target.

The initial compatibility contract should stay small and typed:

- `compatibility.host_api`
- `compatibility.host_version_req`

Why this matters:

- it gives future SDK releases one stable host-facing contract instead of
  relying on implicit crate drift
- it lets the registry and operator surfaces explain why a plugin is visible
  but not activatable
- it keeps older third-party packages from mutating catalog state on hosts they
  do not actually support

Compatibility rules should fail closed:

- absorb must reject incompatible or invalid compatibility declarations before
  provider/channel mutation
- activation planning must surface incompatibility as a first-class blocked
  state
- inventory, discovery, and self-awareness surfaces must reuse the same
  compatibility truth instead of inventing separate heuristics

### 7. Translation And Bootstrap Stay Deterministic

The manifest-first contract should feed the existing translation and bootstrap
pipeline in this order:

1. discover package manifest
2. normalize setup, ownership, and compatibility metadata
3. evaluate host compatibility and ownership blockers
4. translate bridge/runtime profile
5. run security scan and activation planning
6. apply, defer, or block through bootstrap policy

This avoids a future state where setup, bridge translation, and activation
policy each invent their own plugin metadata parsing rules.

### 7.1. Package Manifests Must Be Strict Enough For SDK Authors

The package manifest is the future public contract, so it should be stricter
than the legacy embedded-source compatibility path.

That means:

- package manifests should declare `api_version`
- package manifests should reject unknown top-level fields
- package `version` should parse as valid semver
- legacy embedded source manifests may remain more tolerant during migration,
  but the strict package contract is the thing SDK generators should target

### 8. Untrusted Extensions Stay On Controlled Execution Lanes

This contract should explicitly preserve LoongClaw's preferred extension lanes:

- WASM runtime lane
- process bridge lane
- MCP server lane
- ACP bridge/runtime lanes
- HTTP JSON bridge lane when policy allows it

`NativeFfi` is intentionally not part of that preferred default lane set. The
current bootstrap policy keeps `allow_native_ffi_auto_apply` disabled by
default, because direct FFI bindings weaken the runtime isolation boundary that
WASM, process bridge, MCP, ACP, and policy-allowed HTTP JSON preserve. Native
FFI can remain an explicit operator-controlled opt-in for trusted cases, but it
should not be the default target for third-party plugin packages.

It should explicitly reject the assumption that third-party plugins should run
in-process with the daemon by default.

The package contract is about metadata, discovery, setup, and ownership. It is
not a reason to weaken runtime isolation.

## Recommended Manifest Shape

The initial file contract should stay close to the existing `PluginManifest`
shape and grow additively.

Example:

```json
{
  "api_version": "v1alpha1",
  "plugin_id": "tavily-search",
  "version": "0.1.0",
  "provider_id": "tavily",
  "connector_name": "tavily-http",
  "summary": "Web search provider package for Tavily-backed search.",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search",
    "entrypoint": "https://api.tavily.com/search"
  },
  "setup": {
    "mode": "metadata_only",
    "surface": "web_search",
    "required_env_vars": ["TAVILY_API_KEY"],
    "default_env_var": "TAVILY_API_KEY"
  },
  "compatibility": {
    "host_api": "loongclaw-plugin/v1",
    "host_version_req": ">=0.1.0-alpha.1"
  },
  "slot_claims": [
    {
      "slot": "provider:web_search",
      "key": "tavily",
      "mode": "exclusive"
    }
  ],
  "tags": ["search", "provider", "web"]
}
```

For language-specific bridges such as `process_stdio` or `native_ffi`, the
manifest should also carry `metadata.source_language`. The scaffold command adds
that field when `--source-language` is provided so bridge policy, preflight
summary, and activation planning all stay aligned on one runtime projection.

Important design constraints:

- flat fields used by the current `PluginManifest` remain readable
- nested sections such as `setup`, `compatibility`, and `slot_claims` are
  additive
- package-level `version` stays top-level; `metadata.version` is a migration
  fallback for embedded source manifests, not part of the public package
  contract
- `metadata` remains available for bridge-specific details that do not yet
  deserve first-class schema fields
- package-manifest `metadata` must not use the reserved `plugin_*` namespace
  because that namespace is owned by host-projected runtime and operator state

## Migration Plan

### Phase 1: File Contract Without Breaking Source Markers

- add package-manifest file parsing
- preserve source-marker parsing as a fallback
- define precedence and conflict errors

### Phase 2: Setup Metadata Surfaces

- expose setup metadata to onboarding, install, and doctor
- add guided setup rendering without executing plugin runtime
- introduce governed setup entries only for the cases that need active probing

### Phase 3: Slot- And Compatibility-Aware Activation

- teach activation planning and registry projection about ownership slots plus
  host compatibility fences
- distinguish shared vs exclusive surfaces
- emit typed conflict, incompatibility, and precedence diagnostics

### Phase 4: Supply-Chain And SDK Alignment

- align package contract with trust-tier, signing, and provenance work
- surface typed trust-tier and provenance metadata through scan, activation, and catalog/search
  outputs before signature enforcement lands
- align SDK work with the manifest contract rather than inventing a parallel
  author-facing metadata model

## Relationship To Existing RFCs

This contract should be treated as an upstream architecture layer for:

- `#425` WASM Host Function ABI
- `#426` Plugin SDK Crate

Those RFCs define execution and authoring surfaces. This document defines the
package metadata and ownership contract they should target.

It also supports the broader goals in `#292` without forcing the current
registry-first design to regress into a plugin-owned runtime model.

## Anti-Patterns

The following patterns violate this contract:

- treating source comment extraction as the long-term primary package contract
- requiring plugin runtime execution just to render setup guidance
- inferring exclusive ownership from ids without a declared slot model
- letting plugin manifests directly widen kernel policy or pack boundaries
- copying OpenClaw's in-process trust model into LoongClaw as the default
  extension path
- introducing separate metadata shapes for discovery, setup, translation, and
  SDK authoring

## Validation Standard

Any change that implements this contract should verify:

- manifest discovery precedence and conflict handling
- setup rendering without runtime execution
- slot conflict behavior for exclusive vs shared surfaces
- host compatibility rejection and blocked-inventory visibility
- deterministic translation and bootstrap decisions from the normalized
  manifest
- package-manifest strictness for `api_version`, semver `version`, and unknown
  top-level field rejection
- policy/audit evidence for any governed setup execution path

For doc-only changes, the minimum repository checks should include:

- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

## Future Direction

The long-term target is not "more plugin magic". The target is a plugin
ecosystem that remains:

- discoverable through package metadata
- guided through setup metadata
- governable through slot-aware registry ownership
- evolvable through typed host compatibility contracts
- safe through controlled execution lanes

That is the smaller correct path from today's registry-first baseline to a real
community plugin platform.
