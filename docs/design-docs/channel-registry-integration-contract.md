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
- more config-backed outbound channels
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
paths such as Telegram bot tokens, Feishu webhook secrets, or WeCom AIBot
credentials.

Send and serve requirements must be modeled independently when a channel uses
different outbound and inbound transports. For example, Synology Chat outgoing
webhook sends only need an incoming webhook URL, while a future serve surface
would additionally need outbound webhook verification and allowlist metadata.
Do not collapse those requirements into one merged channel-wide checklist.

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

### 5. Snapshot Builders Are For Concrete Surfaces, Not Just Runtime Loops

A channel only needs a snapshot builder when it has real account-aware state to
report.

That means the registry should cleanly separate:

- runtime-backed channels, which provide snapshot builders and may attach
  background runtime state
- config-backed channels, which provide snapshot builders for config and account
  readiness but do not pretend they have a serve runtime
- stub channels, which only provide catalog metadata

This lets LoongClaw expose shipped send-only integrations without inventing fake
runtime ownership, while still exposing future channels early without
pretending they already have concrete config or runtime support.

Current examples of shipped config-backed outbound surfaces include Discord,
Slack, LINE, DingTalk, WhatsApp, Email, generic Webhook, Google Chat, Signal,
Tlon, Microsoft Teams, Mattermost, Nextcloud Talk, Synology Chat, IRC, and
iMessage / BlueBubbles. They all publish snapshot builders, readiness checks,
and send command metadata without pretending they already own a long-running
serve runtime.

### 6. Runtime Owners Must Bind Through Runtime Registries

Foreground runtime owners such as `multi-channel-serve` must derive their
background channel surface set from registry-backed runtime metadata instead of
hardcoding channel-specific supervisor branches.

That means:

- runtime selection should start from the canonical channel id
- account selectors should use a generic `channel=account` contract
- runner lookup should happen through a channel runner registry keyed by
  canonical ids
- adding a new runtime-backed channel should not require adding new ad hoc CLI
  flags or new supervisor enum variants

This keeps runtime ownership aligned with the same source of truth that powers
catalog, doctor, and inventory views.

### 6.1 Internal SDK Owns Concrete Integration Wiring

LoongClaw now uses an internal compile-time channel SDK at
`crates/app/src/channel/sdk.rs` for channels that have real config, validation,
or background/runtime support.

That SDK is intentionally smaller than a dynamic plugin system. It owns:

- shared `ChannelDescriptor` metadata used by config and migration surfaces
- config enablement and validation hooks for concrete channel integrations
- background runtime eligibility checks for multi-channel supervisors
- the concrete channel integrations that should participate in account
  snapshots and runtime or config status views

This means:

- runtime-backed and config-backed channels must be added through the SDK
  instead of daemon-local registration tables
- `cli` can stay SDK-managed without appearing in the channel catalog
- pure stubs stay registry-only until there is real config or runtime ownership
  to wire up

The goal is to minimize future channel-integration edits without prematurely
committing LoongClaw to a dynamic external plugin boundary.

### 7. High-Quality Stubs Are Valid Platform Entries

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

### 8. Changes Must Stay Additive

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
2. Add an SDK integration descriptor that wires shared config metadata,
   validation, background enablement, and the runtime-backed registry
   descriptor.
3. Implement the runtime snapshot builder that produces
   `ChannelStatusSnapshot` values.
4. Verify that `channel_catalog`, `channel_surfaces`, text rendering,
   `doctor` all pick up the new metadata through shared inventory assembly.
5. Verify that config and multi-channel supervisor flows pick up the new channel
   through the shared SDK instead of daemon-local registration edits.
6. Add regression tests for registry lookup, JSON surfaces, text rendering,
   config/service descriptor order, and doctor behavior.

If the new channel uses an exclusive long-connection transport, the runtime
contract should also make that exclusivity visible in operator status and send
behavior instead of hiding it behind ad hoc command-specific logic.

### Adding A New Config-Backed Channel

When introducing a real send-only or config-only integration without a
background runtime:

1. Add static operation descriptors with capability, availability, doctor, and
   requirement metadata.
2. Add an SDK integration descriptor that wires shared config metadata,
   validation, and enablement for the shipped surface.
3. Add a registry descriptor in the registry integration path so catalog,
   doctor, and operator inventory surfaces can consume the new channel.
4. Implement a snapshot builder that emits per-account config readiness without
   attaching fake runtime state.
5. Mark shipped operations as `implemented` and unshipped runtime operations as
   `stub` or `unsupported`, depending on whether the capability is planned or
   impossible in the current architecture.
6. Split send and serve requirement metadata when the inbound contract needs
   extra secrets, callback validation, or allowlists that the outbound send
   path does not require.
7. Verify that `channel_catalog`, `channel_surfaces`, text rendering, and
   `doctor` all pick up the new metadata through shared inventory assembly.
8. Add regression tests for registry lookup, JSON surfaces, text rendering,
   and config/service descriptor order.

### Adding A New Stub Channel

When the runtime implementation does not exist yet:

1. Add a registry descriptor with `implementation_status=stub`.
2. Define the intended operations and capability flags.
3. Add requirement metadata for known credentials or config inputs when those
   are already part of the intended contract.
4. Do not add placeholder runtime builders or fake health logic.
5. Verify the channel appears correctly in catalog and grouped surfaces.

This is the preferred path for channels such as Nostr, Twitch, Zalo, Zalo
Personal, or WebChat surfaces before real config or runtime support lands.
Channels such as Email, generic Webhook, LINE, DingTalk, Google Chat, Tlon,
Microsoft Teams, Mattermost, Nextcloud Talk, Synology Chat, IRC, and
iMessage / BlueBubbles have already moved beyond this stage and should be
modeled through the config-backed integration path instead.

## Anti-Patterns

The following patterns violate the contract:

- hardcoding channel ids or aliases in daemon CLI rendering
- keeping a second source of truth for doctor requirements
- adding per-channel JSON formatting branches for metadata the registry already
  knows
- adding per-channel supervisor hook fields when runtime-backed runner lookup
  can be keyed by canonical channel id instead
- adding new daemon-local background registration tables for channels that are
  already described by the SDK
- hiding stub channels from catalog surfaces until runtime code exists
- attaching fake runtime-tracking semantics to channels that only have
  config-backed send support
- modeling a shipped long-connection surface as webhook-style static metadata
  after the runtime contract has already moved to account-aware session state

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
without re-breaking the current Telegram/Feishu/Lark/Matrix/WeCom
implementation or forcing broader OpenClaw-style surface coverage to be bolted
on through more hardcoded daemon logic.
