# Plugin SDK Boundary Contract

## Purpose

LoongClaw now has four public plugin architecture layers:

- package truth through the plugin package manifest contract
- foreign-dialect normalization through the OpenClaw compatibility contract
- ecosystem and migration direction through the plugin SDK and ecosystem strategy
- listing and install semantics through the plugin marketplace contract

What still remains underspecified is the next layer between those contracts and
actual author-facing implementation crates:

- which SDK layers exist
- what each SDK layer may expose
- which layer owns guest-side helpers versus host-side extension APIs
- how SDK work should sequence relative to the existing `#426` WASM-oriented SDK
  RFC

This document defines that missing boundary.

It complements, rather than replaces:

- [Plugin Package Manifest Contract](plugin-package-manifest-contract.md)
- [OpenClaw Plugin Compatibility Contract](openclaw-plugin-compatibility-contract.md)
- [Plugin Marketplace And Interface Contract](plugin-marketplace-contract.md)
- [Plugin SDK And Ecosystem Strategy](plugin-sdk-and-ecosystem-strategy.md)

Those documents define package, compatibility, marketplace, and ecosystem
truth. This document defines how author-facing SDK surfaces should be layered on
those truths without creating a second architecture.

## Why This Contract Exists

Issue `#426` already proposes a concrete WASM guest-side SDK crate.
That RFC is valuable, but it is still only one slice of the broader SDK story.
By itself it does not answer:

- whether there should be one SDK crate or a family of SDK layers
- which types belong in a stable contract crate versus bridge-specific helper
  crates
- how future stdio, HTTP JSON, MCP, and compatibility helpers should relate to
  a WASM guest SDK
- how native host extension APIs should stay narrower and more trusted than
  general plugin authoring helpers

Without a boundary contract, the most likely failure mode is not "no SDK".
The likely failure mode is a sprawling SDK surface that:

- re-exports host internals
- duplicates package or preflight metadata models
- blurs guest-side, host-side, and compatibility responsibilities
- makes every future bridge lane invent its own helper vocabulary

This contract exists to prevent that drift before multiple SDK slices land.

## Core Principle

LoongClaw SDKs should help authors target existing contracts.
They should not become new sources of truth.

That means:

- manifests, compatibility, preflight, and marketplace contracts stay
  authoritative
- SDKs provide typed helpers and ergonomics above those contracts
- SDK helpers must not redefine activation, compatibility, or policy truth
- SDK layers must stay small enough that host refactors do not become SDK-wide
  breaking events

## Design Goal

LoongClaw should support an SDK family that is:

- layered instead of monolithic
- native-first but compatible with foreign-package migration flows
- bridge-aware without becoming bridge-fragmented
- explicit about guest-side versus host-side responsibilities
- aligned with package, compatibility, and marketplace contracts

It should not:

- collapse guest-side and host-side APIs into one giant crate
- expose internal host modules as the default authoring surface
- let compatibility helpers redefine package or governance truth
- force every plugin author to depend on the heaviest bridge-specific helper set

## Contract Layers

### 1. SDK Contract Layer

The contract layer is the narrowest and most stable author-facing layer.

It should expose:

- manifest-related schema types that are intentionally author-facing
- setup-related schema types
- bridge request/response envelope types where stable
- preflight, diagnostics, inventory, and summary schema types intended for
  external tooling reuse
- narrow marketplace-facing bindings only where they are explicitly
  listing-scoped and derived from canonical package and governance truth, not as
  a second source of marketplace authority

It must not expose:

- host runtime internals
- app-layer service implementations
- kernel registry implementations
- bridge execution engines
- compatibility policy decisions as mutable helper state

### 2. SDK Runtime Helper Layer

The runtime-helper layer exists to make bridge-style plugins practical to build.

It may expose:

- guest-side WASM helpers
- stdio helper scaffolding
- HTTP JSON helper scaffolding
- MCP helper scaffolding
- structured logging, health, and state helpers
- small bridge-lane utilities that sit above the stable contract layer

It must not:

- override package or activation truth
- silently add capabilities or approval semantics
- bypass bridge policy, preflight, or attestation
- turn one bridge helper into the mandatory dependency for unrelated plugin
  lanes

### 3. Compatibility Helper Layer

The compatibility-helper layer is for foreign ecosystems and migration paths.

It may expose:

- OpenClaw manifest parsing helpers
- migration report builders
- compatibility diagnostics helpers
- shims or adapters that help foreign packages target canonical LoongClaw
  contracts

It must not:

- redefine dialect normalization truth
- become the preferred native authoring surface
- silently widen foreign-package privileges beyond what package and governance
  contracts allow
- flatten native and foreign package identity into one fake neutral model

### 4. Native Host Extension ABI Layer

This is not the same thing as a general-purpose SDK helper crate.
It is the narrowest and most sensitive author-facing extension layer.

It is only for cases that truly need:

- native connector adapters
- native memory adapters
- native runtime adapters
- tightly scoped trusted hook points

This layer must remain:

- trusted
- narrow
- non-default
- explicitly governed

It should be treated as a separate lane from general plugin-author SDKs.

## Boundary Rules

### Rule 1: Package truth stays below SDKs

If package and SDK helper semantics disagree:

- package truth wins
- SDK helpers must adapt
- SDK documentation should be corrected instead of normalizing divergence into
  helper behavior

### Rule 2: Governance truth stays outside SDKs

SDKs may serialize, display, or interpret governance output.
They may not become governance engines.

That means SDKs must not:

- embed private policy evaluators
- decide activation eligibility on their own
- convert advisory marketplace metadata into runtime permissions
- reinterpret attestation validity outside the host's canonical governance flow

### Rule 3: Marketplace truth stays descriptive

SDKs that interact with marketplace data should treat it as listing-layer
metadata.
They must not reinterpret marketplace entries as package, activation, or trust
truth.

### Rule 4: Compatibility helpers stay subordinate to normalization

Compatibility helpers may help authors or migration tooling consume foreign
contracts.
They must still target the same normalized descriptor and compatibility model
that the host uses.

### Rule 5: Helper crates should not force a locked package layout too early

The architectural commitment is to the layer separation, not to one final crate
layout today.
The family of SDK layers may initially land as:

- one crate with internally separated modules
- a few focused crates
- or an incremental sequence starting with one specific bridge helper crate

The important thing is that the layers remain conceptually separate and do not
collapse into one unbounded host-coupled surface.

### Rule 6: Dependency direction must preserve the boundary

Conceptual layering is not only a documentation boundary; it is also a coupling
boundary.

That means:

- the SDK contract layer must not depend on runtime-helper,
  compatibility-helper, or native host-extension ABI layers
- runtime-helper and compatibility-helper layers may depend on the SDK
  contract layer
- compatibility-helper layers must not back-feed foreign-ecosystem abstractions
  into the SDK contract layer
- the native host-extension ABI must not become a default or transitive
  dependency of general plugin-author SDK surfaces

## Recommended SDK Family Shape

The ecosystem strategy document already uses illustrative names.
This boundary contract keeps those names as conceptual slices, not locked
package commitments.

### Conceptual layer: `sdk-contract`

Responsibilities:

- stable shared types
- schema-level serialization contracts
- author-facing manifest/setup/diagnostic vocabulary

Ideal consumers:

- CI tooling
- catalog tooling
- generators and scaffolds
- migration tooling
- external validation and packaging utilities

### Conceptual layer: `sdk-runtime`

Responsibilities:

- bridge-lane helper APIs
- typed I/O helpers
- runtime-state helpers
- structured logging and health helpers

Ideal consumers:

- authors building executable plugins
- bridge-targeted helper packages
- reference examples

### Conceptual layer: `sdk-openclaw-compat`

Responsibilities:

- foreign dialect parsing
- migration and translation helpers
- compatibility diagnostics and adaptation helpers

Ideal consumers:

- migration CLIs
- importer pipelines
- compatibility-aware package authors or maintainers

### Conceptual lane: native host extension ABI

Responsibilities:

- explicit trusted extension points for in-process host integrations

Ideal consumers:

- separately host-approved native packages with deeper host requirements
- future curated internal extension packages

Compatibility-origin packages and compatibility shims must not target the native
host-extension ABI directly unless they are first re-authored or repackaged as
native LoongClaw packages and pass separate host governance for that lane.

## Relationship To `#426`

Issue `#426` should now be read as one implementation slice inside this broader
boundary contract.

Specifically, `#426` maps most naturally onto:

- the **runtime-helper layer**
- for the **WASM guest-side lane**

That means the WASM guest SDK should be treated as:

- a concrete bridge-targeted helper package
- not as the total plugin SDK story
- not as the owner of manifest or marketplace truth
- not as the default shape for host-native extension APIs

## Recommended Execution Order

### Phase 1: Boundary-first planning

Before more helper crates appear, the system should have explicit answers for:

- which types belong to the stable contract layer
- which helper APIs are bridge-specific
- which lanes are compatibility-only
- which lanes require trusted host-extension treatment

This document provides that answer.

### Phase 2: Narrow stable contract extraction

If shared schema types are needed externally, introduce a minimal contract layer
or contract module surface first.
This should happen before expanding bridge helper crates.

### Phase 3: Bridge-targeted helper slices

Land helper APIs lane by lane.
The most defensible order is:

1. WASM guest-side helper surface (`#426` scope)
2. stdio or HTTP JSON helper surfaces if and when their execution lanes become
   author-facing
3. MCP helper surfaces where the host contract is stable enough

### Phase 4: Compatibility helper slices

OpenClaw or future foreign-package helper surfaces should land only after the
canonical package and runtime-helper layers are clear enough that compatibility
code has a stable target.

### Phase 5: Native host extension ABI

Trusted in-process extension lanes should stay later and narrower than the
bridge-targeted helper surfaces.
They are not the general plugin-author default path.

## Illustrative Responsibilities Matrix

| Layer | Owns stable types? | Owns helper ergonomics? | Owns governance truth? | Default authoring lane? |
|------|---------------------|--------------------------|------------------------|-------------------------|
| SDK contract layer | Yes | Minimal | No | Yes |
| SDK runtime helper layer | No | Yes | No | Yes |
| Compatibility helper layer | No | Yes | No | No |
| Native host extension ABI | Narrow, trusted subset only | Narrow | No | No |

This matrix is the boundary in one glance:

- contract layers carry shared truths
- helper layers provide ergonomics
- governance stays with the host
- compatibility and host-native lanes stay specialized instead of becoming the
  default path

## Anti-Patterns

The following patterns violate this contract:

- treating the WASM guest SDK as the only plugin SDK the project will ever
  need
- re-exporting host-internal app or kernel modules as the default SDK surface
- putting marketplace policy logic inside runtime helper crates
- letting compatibility helper crates redefine normalized descriptor or
  activation semantics
- mixing trusted host-native extension APIs into the same default crate that is
  meant for ordinary bridge-targeted plugin authors
- duplicating package-manifest, marketplace, or preflight schemas inside helper
  crates instead of consuming the canonical contracts

## Validation Standard

Any implementation that claims conformance with this boundary contract should
verify:

- stable separation between contract and helper layers
- no helper crate becomes a second policy or activation engine
- foreign compatibility helpers still target canonical normalized truth
- host-native extension lanes stay narrower and more trusted than general
  plugin authoring surfaces
- future SDK work references this boundary when deciding whether a new helper
  belongs in contract, runtime-helper, compatibility-helper, or host-native
  lanes

For doc-only changes, the minimum repository checks should include:

- `cargo fmt --all -- --check`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

## Relationship To Existing Documents

This boundary contract should be treated as the explicit bridge between:

- package truth (`plugin-package-manifest-contract.md`)
- compatibility truth (`openclaw-plugin-compatibility-contract.md`)
- marketplace truth (`plugin-marketplace-contract.md`)
- ecosystem strategy (`plugin-sdk-and-ecosystem-strategy.md`)
- concrete helper implementation slices such as `#426`

Those layers already existed conceptually.
This document makes their author-facing boundaries explicit.

## Non-Goals

This contract does not:

- implement the `#426` WASM guest SDK
- define a final crates.io publishing plan
- define all future bridge helper APIs in detail
- specify exact proc-macro syntax
- choose a final permanent crate layout for every SDK layer
- replace the existing package, compatibility, or marketplace contracts

Those are follow-on implementation and packaging tasks.

## Future Direction

The long-term target is an SDK family that remains:

- contract-driven instead of helper-driven
- bridge-aware instead of bridge-fragmented
- native-first while still supporting foreign migration lanes
- small enough that host refactors do not automatically become ecosystem-wide
  breakages
- explicit enough that future issues can say "this belongs to the contract
  layer" or "this is a runtime-helper concern" without reopening the whole
  architecture question

That gives LoongClaw a clean next step after the marketplace contract:

- package truth is defined
- compatibility truth is defined
- marketplace truth is defined
- SDK boundaries are now defined

The next implementation slices can therefore land against explicit boundaries
instead of inventing new ones.
