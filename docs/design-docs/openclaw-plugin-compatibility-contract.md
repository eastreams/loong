# OpenClaw Plugin Compatibility Contract

## Purpose

LoongClaw wants two things at the same time:

- strong compatibility with the existing OpenClaw plugin ecosystem
- a kernel that stays small, governable, and fail-closed

Those goals are compatible only if foreign plugin ecosystems enter through one
explicit seam instead of leaking dialect-specific rules across scan,
translation, activation, bootstrap, and runtime execution.

This document defines that seam.

## Design Goal

The compatibility model must let LoongClaw ingest:

- native LoongClaw plugins
- modern OpenClaw manifest packages
- legacy OpenClaw `package.json` extension packages
- future foreign plugin dialects

without turning the kernel into a pile of ecosystem-specific special cases.

The required properties are:

- native LoongClaw stays the primary first-class contract
- foreign dialects are recognized explicitly, never implicitly
- compatibility is runtime-gated, not discovery-auto-enabled
- polyglot execution remains a bridge/runtime concern, not a kernel dialect
  concern
- unsupported compatibility modes fail closed before activation

## Core Principle

LoongClaw should not "become OpenClaw" in order to support OpenClaw plugins.

Instead, the system should normalize foreign dialects into one canonical
LoongClaw descriptor model:

1. discover the incoming dialect
2. classify it explicitly
3. project it into a canonical descriptor
4. carry dialect provenance forward
5. require runtime support for the compatibility mode before activation

That keeps compatibility strong while keeping architectural complexity local.

## Contract Layers

### 1. Discovery Layer

Discovery recognizes three package-level contracts:

- `loongclaw.plugin.json`
- `openclaw.plugin.json`
- `package.json` with `openclaw.extensions`

The scan layer does not activate anything. It only identifies the contract,
derives the package root, resolves entry hints, and records provenance.

### 2. Descriptor Layer

Every discovered plugin is projected into one canonical `PluginDescriptor`
shape with three boundary fields:

- `dialect`
- `dialect_version`
- `compatibility_mode`

Current dialects are:

- `loongclaw_package_manifest`
- `loongclaw_embedded_source`
- `openclaw_modern_manifest`
- `openclaw_legacy_package`

Current compatibility modes are:

- `native`
- `openclaw_modern`
- `openclaw_legacy`

This is the key containment move: foreign ecosystem identity is preserved, but
the rest of the pipeline still consumes one normalized descriptor contract.

### 3. Translation Layer

Translation remains bridge-oriented, not ecosystem-oriented.

That means:

- TypeScript, JavaScript, Python, and other languages are mapped by runtime
  bridge needs such as `process_stdio`, `http_json`, `mcp_server`, or future
  bridges
- OpenClaw compatibility does not get a custom execution plane
- the descriptor still becomes one `PluginIR`

Language flexibility therefore lives here:

- JS/TS commonly project to `process_stdio`, `http_json`, or MCP-like bridges
- Python commonly projects to `process_stdio` or service bridges
- future languages can join by implementing a supported bridge/adapter profile

The kernel does not need separate "JS plugin", "Python plugin", or
"OpenClaw-native runtime" code paths.

### 4. Activation Layer

Activation is where fail-closed governance applies.

`BridgeSupportMatrix` now governs:

- supported bridges
- supported adapter families
- supported compatibility modes
- supported compatibility shims
- optional compatibility shim support profiles

Default runtime behavior is intentionally conservative:

- `native` compatibility is supported
- OpenClaw compatibility modes are not supported unless the runtime opts in
- foreign compatibility shims must also be enabled explicitly; mode allowlists
  alone are not enough
- when a shim support profile is declared, activation must also satisfy that
  profile's dialect, bridge, adapter family, and source-language constraints

This makes compatibility an explicit host decision instead of a side effect of
successful discovery.

### 5. Governance Layer

Preflight and inventory surfaces expose the boundary truth so operators and CI
can reason about compatibility without re-parsing manifests:

- dialect
- dialect version
- compatibility mode
- selected compatibility shim support profile
- foreign dialect diagnostics
- legacy OpenClaw diagnostics
- compatibility shim requirements
- compatibility shim support-profile mismatches

This produces one important separation:

- scan tells us what the package is
- runtime policy tells us whether this host may activate it

The profile layer is intentionally narrow. It does not create language-specific
kernel branches. It only lets the host declare:

- which shim is enabled
- which foreign dialects that shim can translate
- which bridge kinds and adapter families that shim may project onto
- which source-language families that shim is allowed to front

That keeps the core architecture stable while still letting LoongClaw absorb
OpenClaw new/legacy packages and future polyglot ecosystems behind one
canonical runtime boundary.

Inventory, tool-search, and preflight now surface the selected shim-support
profile and any mismatch reasons directly from activation truth. Operator-facing
governance therefore does not need to re-derive compatibility state from raw
manifests or host-local heuristics.

Runtime execution now also revalidates the canonical compatibility projection
before bridge dispatch. That means a provider that was manually patched,
drifted, or registered with inconsistent compatibility metadata cannot slip
past activation-time policy just because it already exists in the catalog.
Compatibility-profile violations are treated as boundary breaches, not soft
runtime warnings, and therefore fail closed at connector invocation time.

### 6. Activation Attestation Layer

Ready plugins now stamp an attested activation contract into absorbed provider
metadata:

- `plugin_activation_contract_json`
- `plugin_activation_contract_checksum`

This contract is the runtime-approved compatibility truth for the plugin at the
moment it crossed activation.

The runtime does not trust that metadata blindly. Before bridge dispatch it:

- verifies the attested checksum
- rejects malformed or partial attestation metadata
- compares the current provider projection against the attested activation
  contract
- re-checks the attested contract against the current host compatibility matrix

That gives LoongClaw one explicit compatibility seam:

- discovery tells us what was found
- activation tells us what was approved
- attestation preserves what was approved
- runtime proves the provider still matches that approval

That truth is no longer runtime-only. Operator-facing surfaces now expose the
same attestation state for loaded plugins:

- `plugin_inventory`
- `tool_search`
- `plugin_preflight`

Loaded providers therefore report whether their activation contract is:

- `verified`
- `missing`
- `invalid`

Preflight governance now also emits structured operator actions alongside
human-readable remediation summaries. That action contract identifies:

- a stable `action_id` that external operator tooling can persist or de-dup on
- which operator surface owns the fix, such as host runtime, bridge policy, or
  plugin package
- which plugin/provider instance the action targets
- whether runtime reload or re-absorb is required
- which follow-up preflight profile should be rerun after the fix

Preflight summary output now also emits a deduplicated
`operator_action_plan` and aggregates those actions by surface and kind using
distinct action targets, so operator tooling can distinguish runtime quarantine
work from package migration work or bridge-policy work without reprocessing
every result row.

The daemon-facing `loongclaw plugins preflight` and `loongclaw plugins actions`
commands are intentionally thin wrappers over that same preflight contract.
They do not introduce a second compatibility or governance engine. This keeps
OpenClaw new/legacy compatibility, TS/Py/JS polyglot intake, and future host
automation aligned on one source of truth instead of letting each CLI or SDK
re-derive policy from raw manifests.

Those machine-readable command outputs now also expose one stable
`bridge_support_provenance` object at the command level and inside the emitted
preflight summary. SDKs and automation therefore do not need to correlate
separate top-level fields just to answer "which bridge policy actually governed
this evaluation?" They can bind to one nested provenance contract while older
flat fields remain available for backward compatibility.

Those machine-readable outputs are now also explicitly versioned. `run-spec`
reports, `plugin_preflight` summaries, and `loongclaw plugins` command JSON
payloads all carry a standard `schema` object with stable `version`, `surface`,
and `purpose` metadata, while also retaining `schema_version` as a flat
compatibility alias. TS/Py/JS SDKs, host automation, and future remote APIs can
therefore negotiate additive upgrades without inferring compatibility from
daemon build versions or ad hoc field presence.

Those commands now also support checksum-pinnable bundled bridge support
profiles. The first two host-facing presets are:

- `native-balanced`
- `openclaw-ecosystem-balanced`

The OpenClaw preset enables modern and legacy OpenClaw compatibility only
through an explicit shim-supported runtime matrix. Native plugins still flow
through the same bridge-first contract, so TS/Py/JS and broader language
ecosystems do not require language-specific kernel branches just because one
host opted into OpenClaw compatibility.

Hosts do not need to hard-code those preset names. `loongclaw plugins
bridge-profiles` now exposes the bundled compatibility presets, their
checksums, enabled compatibility modes, shims, and shim support profiles
directly through one machine-readable operator surface.

`loongclaw plugins bridge-template` now also supports two operator outputs from
the same preflight recommendation:

- a full materialized bridge support template via `--output`
- a minimal checksum-addressable delta artifact via `--delta-output`

That split matters operationally. Hosts that want a turnkey profile can adopt
the full template, while hosts that already maintain a native-first runtime
baseline can persist only the minimal compatibility delta that needs to be
layered on top. OpenClaw compatibility and future TS/Py/JS or broader language
ecosystems therefore stay additive at the host boundary instead of forcing
every runtime to vendor a growing copy of full bridge policy JSON.

That delta output is not write-only. The same operator surfaces can now accept
`--bridge-support-delta` as an active bridge input and will materialize the
effective runtime policy from the bundled base profile plus the signed delta
artifact before preflight or action planning runs. This keeps host-local policy
composition explicit and checksum-verifiable without teaching the kernel about
"OpenClaw mode", "Python mode", or any other ecosystem-specific runtime branch.

The same composition model now also reaches generic spec execution. JSON specs
can carry a `bridge_support_selection` envelope that points at a bundled base
profile, full policy file, or delta artifact input, and `run-spec` resolves
that envelope into one concrete `bridge_support` policy before execution. That
lets sandbox, CI, staged rollout, and release spec artifacts reuse the same
compatibility composition contract that plugin governance already uses.

That resolution is also observable in the final `run-spec` report itself. The
daemon now stamps the emitted JSON with:

- `bridge_support_source`
- `bridge_support_delta_source`
- `bridge_support_delta_sha256`

alongside the already computed effective policy checksum and sha256. A spec run
therefore proves both which compatibility policy actually governed execution
and whether that policy came from a full host-authored file, a bundled profile,
or a bundled-profile-plus-delta composition.

For one-off rollout control, `run-spec` also accepts explicit bridge override
flags at invocation time. Those overrides are allowed only when the spec file
does not already declare bridge support inline or through its own selection
envelope, so the operator still has one unambiguous compatibility source of
truth for each run.

When a host is already running a custom delta-derived bridge policy that fully
covers the scanned ecosystem, preflight now recognizes that active support as
aligned and suppresses repeat "author bridge profile delta" recommendations.
Operators therefore get one stable closed loop:

- derive the minimum additive compatibility patch
- persist and pin it as an artifact
- feed that artifact back into preflight/actions/runtime governance
- avoid churn once the active custom policy already satisfies the ecosystem

`runtime_activation` preflight treats `missing` or `invalid` loaded attestation
as an actual governance problem instead of a cosmetic warning. If a loaded
provider no longer proves the contract it crossed activation with, operators
must quarantine it from the active catalog and then repair or re-absorb it
before treating the runtime as ready.

This matters for future TS, JS, Python, and broader language ecosystems. The
kernel does not need to remember ecosystem-specific rules after activation; it
only needs to verify one canonical attested contract plus one current runtime
projection.

Preflight summary output now also reports ecosystem distribution by source kind,
dialect, compatibility mode, source language, and bridge kind. That gives hosts
one compact view of whether they are still mostly native, whether OpenClaw
compatibility is concentrated in modern or legacy packages, and whether their
language mix still fits the shim support profile they intended to enable.

## Mapping Rules

### Native LoongClaw

`loongclaw.plugin.json` maps to:

- `dialect = loongclaw_package_manifest`
- `compatibility_mode = native`

Embedded source markers map to:

- `dialect = loongclaw_embedded_source`
- `compatibility_mode = native`

### Modern OpenClaw

`openclaw.plugin.json` maps to:

- `dialect = openclaw_modern_manifest`
- `compatibility_mode = openclaw_modern`

The package is normalized into a canonical LoongClaw manifest/descriptor, but
its foreign provenance remains attached for policy and runtime decisions.

### Legacy OpenClaw

`package.json#openclaw.extensions` maps to:

- `dialect = openclaw_legacy_package`
- `compatibility_mode = openclaw_legacy`

Legacy packages are intentionally treated as compatibility-only inputs. They
should not silently gain first-class status just because they can be parsed.

## Why Compatibility Mode Is Runtime-Gated

A parsed plugin is not a trusted plugin.

If compatibility modes were enabled automatically when discovery succeeded, the
host would accidentally grant trust to:

- foreign lifecycle semantics
- foreign setup expectations
- foreign package assumptions
- foreign runtime affordances

That would make kernel behavior implicit and hard to audit.

The explicit compatibility-mode gate keeps the trust decision in one place.

This is especially important for future adapter shims that may need:

- command allowlists
- sandboxing rules
- bridge-specific runtime wrappers
- provider/channel projection policies
- versioned compatibility behavior

## Why This Stays Architecturally Cheap

The compatibility design is deliberately narrow:

- discovery learns foreign manifests
- descriptor records dialect provenance
- activation checks compatibility mode support
- runtime shims are opt-in

Everything else stays on the existing canonical path.

This avoids:

- dialect-specific branches across the whole kernel
- duplicated translation logic for every ecosystem
- separate inventory/search/governance stacks per plugin family
- language-specific kernel branches

The cost of compatibility is therefore additive and localized.

## Polyglot Ecosystem Direction

LoongClaw should support broader plugin ecosystems through bridge contracts, not
through hard-coded language privilege.

The scalable model is:

- a plugin package declares contract metadata
- translation projects it to a supported bridge/runtime profile
- the bridge owns process, protocol, and language details

This scales to:

- TypeScript / JavaScript
- Python
- Rust
- Go
- Wasm component packages
- remote MCP-style services
- future HTTP or ACP-style runtimes

The kernel only needs to know:

- what the plugin claims
- what bridge/adapter it needs
- whether the runtime explicitly supports that combination

## Policy Defaults

Balanced policy defaults should reflect both openness and safety:

- runtime activation blocks unsupported activation states and blocking runtime
  diagnostics
- runtime execution re-checks compatibility contracts against the same support
  matrix before dispatch
- attested activation contracts must verify and match current provider
  projection before dispatch
- modern foreign dialects may be visible without being treated as native
- legacy OpenClaw contracts are stricter in release/submission lanes
- missing compatibility shims are release blockers

That gives LoongClaw a controlled migration path:

- ecosystem-compatible by discovery
- explicit by activation
- conservative by runtime default

## Future Extensions

This model generalizes cleanly to other ecosystems.

A future dialect should add:

- one new dialect enum value
- one compatibility mode if runtime shimming is needed
- one normalizer from foreign manifest to canonical descriptor
- one runtime shim entry in the compatibility matrix

It should not require a new execution architecture.

Examples of future growth:

- marketplace importers for third-party plugin registries
- richer shim families for OpenClaw modern and legacy packages
- signed compatibility bundles
- dialect-specific migration CLIs that emit native `loongclaw.plugin.json`
- per-compatibility-mode sandbox presets

## Non-Goals

This contract does not:

- make foreign dialects equal to native by default
- auto-enable OpenClaw execution on every host
- guarantee zero-effort execution for every foreign plugin package
- put language-specific semantics into the kernel
- replace bridge/runtime policy with manifest trust

## Decision

LoongClaw should support OpenClaw and broader polyglot ecosystems through:

- explicit dialect discovery
- canonical descriptor normalization
- provenance-preserving inventory/governance
- runtime-gated compatibility modes
- bridge-first language execution

That is the smallest architecture that is both ecosystem-open and kernel-safe.
