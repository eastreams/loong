# Channel Registry Integration Contract

## Purpose

LoongClaw's channel platform is moving toward a registry-first model where every
operator-facing surface derives from one shared metadata seam instead of
re-encoding channel knowledge in CLI commands, doctor checks, or per-channel
runtime entrypoints.

This document defines the contract for adding or evolving channels after the
current registry/capability/availability/doctor/requirement refactors.

## Why This Contract Exists

The original Telegram and Feishu/Lark implementation started as hand-wired
runtime paths. That was acceptable while LoongClaw only needed two concrete
channels, but it does not scale to:

- more runtime-backed channels
- higher-quality stubs for not-yet-implemented channels
- machine-readable operator surfaces
- future plugin or hotplug expansion

OpenClaw already treats channel metadata as first-class product surface, but its
metadata is distributed across plugin package metadata, registry ordering,
configuration schema, and capability probes. LoongClaw intentionally uses a
smaller Rust-native contract today: one compile-time registry descriptor layer
that can feed all current operator surfaces consistently.

That tradeoff keeps the design boring and additive while still preserving the
important architectural lesson from OpenClaw: channel metadata must have an
explicit source of truth.

## Contract

### 1. Registry Owns Channel Identity

Channel identity must be declared exactly once in the app registry.

The registry is responsible for:

- canonical `id`
- `label`
- selection ordering metadata
- selection-facing summary text
- `aliases`
- `transport`
- implementation status
- capability flags
- supported operations

No caller should hardcode alias normalization, transport names, or channel
selection labels outside the registry.

### 2. Operation Metadata Is Static And Declarative

Each channel operation must be described by static metadata, not by ad hoc CLI
logic.

Current required fields are:

- operation `id`
- operator-facing `label`
- CLI `command`
- `availability`
- `tracks_runtime`
- supported target kinds
- static `requirements`

Requirement metadata exists to describe what the operation needs before runtime
state is even considered. That includes config keys and environment-pointer
paths such as Telegram bot tokens or Feishu webhook secrets.

Target-kind metadata exists to describe the operator contract for each command
without pretending every surface routes through a conversation id. Some planned
surfaces need `address` or `endpoint` targets even before a runtime adapter
exists.

### 3. Doctor Metadata Lives Next To Operation Metadata

If an operation needs doctor coverage, the trigger metadata belongs beside the
operation descriptor rather than in a second parallel table.

This prevents drift between:

- what an operation is called
- whether it is available
- what it requires
- what doctor checks should be emitted

If an operation does not need doctor output yet, that should be represented as
empty doctor metadata instead of implicit caller-side special cases.

### 4. Operator Surfaces Must Derive From Inventory

Operator-facing channel surfaces should be projections of shared inventory data,
not separate implementations.

Current projections are:

- `channel_catalog`
- `channel_surfaces`
- `channels` text output
- `doctor` channel checks

When adding metadata to a channel, the desired flow is:

1. extend registry descriptors
2. extend shared inventory/catalog structs
3. let JSON/text/doctor surfaces consume those structs

Do not start by teaching each CLI surface the new metadata independently.

### 5. Runtime Builders Are Only For Runtime-Backed Channels

A channel only needs a runtime snapshot builder when it has real account-aware
or runtime-aware state to report.

That means the registry should cleanly separate:

- runtime-backed channels, which provide snapshot builders
- stub channels, which only provide catalog metadata

This lets LoongClaw expose future channels early without pretending they already
have runtime support.

### 6. High-Quality Stubs Are Valid Platform Entries

A stub channel is still expected to be a first-class catalog entry.

High-quality stubs should include:

- stable canonical id and aliases
- selection order, selection label, and short blurb
- transport family
- operation list
- capability flags
- implementation status
- supported target kinds
- requirement metadata when known

This keeps future channels visible to operators and avoids later invasive
migration when the runtime implementation arrives.

### 7. Changes Must Stay Additive

Channel-platform evolution must preserve existing public surfaces whenever
possible.

In practice that means:

- prefer adding new catalog fields over renaming or deleting old ones
- keep legacy JSON views alive while introducing newer grouped views
- avoid changing CLI semantics unless a regression test proves the need

The registry contract is intended to absorb new metadata without breaking older
consumers.

## Integration Recipes

### Adding A New Runtime-Backed Channel

When introducing a new real channel implementation:

1. Add static operation descriptors with capability, availability, doctor, and
   requirement metadata.
2. Add a registry descriptor with canonical id, aliases, transport, and runtime
   builder.
3. Implement the runtime snapshot builder that produces
   `ChannelStatusSnapshot` values.
4. Verify that `channel_catalog`, `channel_surfaces`, text rendering, and
   `doctor` all pick up the new metadata through shared inventory assembly.
5. Add regression tests for registry lookup, JSON surfaces, text rendering, and
   doctor behavior.

### Adding A New Stub Channel

When the runtime implementation does not exist yet:

1. Add a registry descriptor with `implementation_status=stub`.
2. Define the intended operations and capability flags.
3. Add requirement metadata for known credentials or config inputs when those
   are already part of the intended contract.
4. Do not add placeholder runtime builders or fake health logic.
5. Verify the channel appears correctly in catalog and grouped surfaces.

This is the preferred path for channels such as Discord, Slack, LINE, WeCom,
DingTalk, WhatsApp, Email, or generic Webhook surfaces before full runtime
support lands.

## Anti-Patterns

The following patterns violate the contract:

- hardcoding channel ids or aliases in daemon CLI rendering
- keeping a second source of truth for doctor requirements
- adding per-channel JSON formatting branches for metadata the registry already
  knows
- hiding stub channels from catalog surfaces until runtime code exists
- introducing runtime builders for channels that have no runtime state

## Validation Standard

Any registry contract change should verify the same path LoongClaw CI enforces:

- `cargo fmt --all --check`
- `git diff --check`
- `./scripts/check_architecture_drift_freshness.sh docs/releases/architecture-drift-$(date -u +%Y-%m).md`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features -- --test-threads=1`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh` for doc-only or doc-touching changes

## Current Scope And Future Direction

This contract is intentionally smaller than OpenClaw's broader plugin-driven
channel ecosystem.

LoongClaw does not yet need:

- external channel plugin loading
- provider-discovered runtime capability probes
- a trait-heavy multi-backend channel substrate

It does need a stable metadata seam that allows those future steps to be added
without re-breaking the current Telegram/Feishu/Lark implementation or forcing
Discord/Slack to be bolted on through more hardcoded daemon logic.
